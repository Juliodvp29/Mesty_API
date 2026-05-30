use crate::repositories::user::PostgresUserRepository;
use domain::user::entity::User;
use domain::user::repository::UserRepository;
use domain::user::value_objects::{PhoneNumber, Username};
use sqlx::PgPool;

const TEST_SECRET: &[u8] = b"test_phone_hash_secret";

#[sqlx::test(migrations = "../../migrations")]
async fn test_create_user(pool: PgPool) -> sqlx::Result<()> {
    let repo = PostgresUserRepository::new(pool.clone(), None);
    let phone = PhoneNumber::new("+573001111111".to_string()).unwrap();

    let user = User::new(
        Some(Username::new("testuser_create".to_string()).unwrap()),
        phone.clone(),
        Some(domain::user::value_objects::Email::new("test@example.com".to_string()).unwrap()),
        TEST_SECRET,
    );

    repo.create(&user).await.expect("Should create user");
    let user_id = user.id.clone();

    let found = repo.find_by_id(&user_id).await.expect("Should find user");
    assert!(found.is_some());
    assert_eq!(found.unwrap().phone.as_str(), "+573001111111");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_find_by_phone(pool: PgPool) -> sqlx::Result<()> {
    let repo = PostgresUserRepository::new(pool.clone(), None);
    let phone = PhoneNumber::new("+573001222222".to_string()).unwrap();

    let user = User::new(
        Some(Username::new("testuser_phone".to_string()).unwrap()),
        phone.clone(),
        None,
        TEST_SECRET,
    );

    repo.create(&user).await.expect("Should create user");

    let found = repo
        .find_by_phone(&phone)
        .await
        .expect("Should find by phone");
    assert!(found.is_some());
    assert_eq!(found.unwrap().phone.as_str(), "+573001222222");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_update_user(pool: PgPool) -> sqlx::Result<()> {
    let repo = PostgresUserRepository::new(pool.clone(), None);
    let phone = PhoneNumber::new("+573001333333".to_string()).unwrap();

    let user = User::new(
        Some(Username::new("testuser_update".to_string()).unwrap()),
        phone.clone(),
        None,
        TEST_SECRET,
    );

    repo.create(&user).await.expect("Should create user");

    let mut found = repo
        .find_by_phone(&phone)
        .await
        .expect("Should find user")
        .unwrap();
    found.status_text = "Updated status".to_string();
    repo.update(&found).await.expect("Should update user");

    let updated = repo
        .find_by_phone(&phone)
        .await
        .expect("Should find updated user")
        .unwrap();
    assert_eq!(updated.status_text, "Updated status");

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_delete_soft_user(pool: PgPool) -> sqlx::Result<()> {
    let repo = PostgresUserRepository::new(pool.clone(), None);
    let phone = PhoneNumber::new("+573001444444".to_string()).unwrap();

    let user = User::new(
        Some(Username::new("testuser_delete".to_string()).unwrap()),
        phone.clone(),
        None,
        TEST_SECRET,
    );

    repo.create(&user).await.expect("Should create user");
    let user_id = user.id.clone();

    repo.delete_soft(&user_id)
        .await
        .expect("Should soft delete user");

    let found = repo
        .find_by_id(&user_id)
        .await
        .expect("Should not find deleted user");
    assert!(found.is_none());

    Ok(())
}
