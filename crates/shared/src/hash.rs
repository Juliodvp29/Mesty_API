use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Genera el hash HMAC-SHA256 del número de teléfono usando la clave secreta del servidor.
///
/// Este es el método principal y seguro. Usa `secret = config.server.phone_hash_secret.as_bytes()`.
/// Protege contra rainbow tables porque el secreto actúa como sal estática de servidor.
pub fn hash_phone(phone: &str, secret: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC acepta cualquier tamaño de clave");
    mac.update(phone.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Genera el hash SHA-256 simple del número de teléfono (SIN sal/secreto).
///
/// ⚠️ Solo debe usarse durante la migración de inicio para detectar registros
/// que aún usan el hash antiguo y rehashearlos con HMAC-SHA256.
/// No usar en código nuevo.
pub fn hash_phone_sha256(phone: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(phone.as_bytes());
    hex::encode(hasher.finalize())
}
