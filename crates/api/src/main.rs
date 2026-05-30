use api::routes::create_router;
use api::services::create_metrics;
use axum::extract::connect_info::IntoMakeServiceWithConnectInfo;
use redis::Client;
use redis::aio::ConnectionManager;
use shared::config::Config;
use shared::hash::hash_phone;
use shared::logging::{AppEnv, init_logging};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let config = Config::load().map_err(|e| {
        eprintln!("Error loading configuration: {}", e);
        std::process::exit(1);
    })?;

    init_logging(AppEnv::from(config.app_env.as_str()));

    tracing::info!("Starting messenger backend");
    tracing::info!("Environment: {:?}", config.app_env);

    let db_pool = PgPoolOptions::new()
        .max_connections(config.database.max_connections)
        .min_connections(config.database.min_connections)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .idle_timeout(std::time::Duration::from_secs(600))
        .test_before_acquire(true)
        .connect(&config.database.url)
        .await
        .expect("Could not connect to PostgreSQL");

    let redis_client = Client::open(config.redis.url.clone()).expect("Invalid Redis URL");
    let redis_manager = ConnectionManager::new(redis_client.clone())
        .await
        .expect("Could not connect to Redis");

    // Migrar phone_hashes de SHA-256 plano → HMAC-SHA256 si el secreto está configurado.
    if let Err(e) =
        rehash_phone_numbers(&db_pool, &redis_manager, &config.server.phone_hash_secret).await
    {
        tracing::error!("phone_hash rehash migration failed: {:?}", e);
    }

    let metrics = create_metrics().expect("Failed to create metrics");

    // Start background metrics worker
    let metrics_clone = metrics.clone();
    let db_pool_clone = db_pool.clone();
    let redis_client_clone = redis_client.clone();
    tokio::spawn(async move {
        loop {
            {
                let m = metrics_clone.read();
                // SQLx metrics
                m.db_pool_active.set(db_pool_clone.size() as i64);
                m.db_pool_idle.set(db_pool_clone.num_idle() as i64);

                // Redis metrics
                if let Ok(mut conn) = redis_client_clone.get_connection() {
                    let info: Result<String, _> =
                        redis::cmd("INFO").arg("clients").query(&mut conn);
                    if let Ok(info_str) = info {
                        for line in info_str.lines() {
                            if let Some(count) = line
                                .strip_prefix("connected_clients:")
                                .and_then(|s| s.trim().parse::<i64>().ok())
                            {
                                m.redis_connected_clients.set(count);
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
    });

    // Start push notification worker
    let worker_redis = redis_manager.clone();
    let worker_pool = db_pool.clone();
    let worker_config = config.push.clone();
    tokio::spawn(async move {
        api::services::push::push_notification_worker(worker_redis, worker_pool, worker_config)
            .await;
    });

    let app: IntoMakeServiceWithConnectInfo<_, SocketAddr> =
        create_router(&config, db_pool, redis_manager, metrics)
            .into_make_service_with_connect_info();

    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    println!("Server listening on {}", addr);
    println!("Environment: {:?}", config.app_env);

    axum::serve(listener, app).await?;

    Ok(())
}

/// Migra registros de usuarios cuyo `phone_hash` aún use SHA-256 plano al nuevo HMAC-SHA256.
///
/// La detección se hace comparando `encode(digest(phone, 'sha256'), 'hex')` con el
/// `phone_hash` almacenado. Si coinciden, el registro usa el formato antiguo.
/// La función carga el teléfono en texto claro (ya almacenado en la columna `phone`),
/// recalcula el HMAC-SHA256 con el secreto de servidor y actualiza la fila.
async fn rehash_phone_numbers(
    pool: &sqlx::PgPool,
    redis: &ConnectionManager,
    secret: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Seleccionar usuarios donde phone_hash coincide con el SHA-256 plano (formato antiguo)
    let rows = sqlx::query!(
        r#"
        SELECT id, phone, phone_hash
        FROM users
        WHERE deleted_at IS NULL
          AND phone_hash IS NOT NULL
          AND phone_hash = encode(digest(phone, 'sha256'), 'hex')
        "#
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        tracing::info!("phone_hash rehash: no legacy SHA-256 records found, nothing to migrate.");
        return Ok(());
    }

    tracing::info!(
        "phone_hash rehash: found {} record(s) with legacy SHA-256 hash, migrating…",
        rows.len()
    );

    let secret_bytes = secret.as_bytes();
    let mut redis_conn = redis.clone();

    for row in rows {
        let old_hash = row.phone_hash.clone().unwrap_or_default();
        let new_hash = hash_phone(&row.phone, secret_bytes);

        // Actualizar Postgres
        sqlx::query!(
            "UPDATE users SET phone_hash = $1 WHERE id = $2",
            new_hash,
            row.id
        )
        .execute(pool)
        .await?;

        // Actualizar Redis set phone_hashes: remover viejo, insertar nuevo
        use redis::AsyncCommands;
        let _: () = redis_conn
            .srem("phone_hashes", &old_hash)
            .await
            .unwrap_or(());
        let _: () = redis_conn
            .sadd("phone_hashes", &new_hash)
            .await
            .unwrap_or(());

        tracing::debug!(
            user_id = %row.id,
            "phone_hash rehash: migrated {} → {}",
            &old_hash[..8],
            &new_hash[..8]
        );
    }

    tracing::info!("phone_hash rehash: migration complete.");
    Ok(())
}
