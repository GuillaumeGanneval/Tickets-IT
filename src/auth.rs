use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
    response::{IntoResponse, Redirect, Response},
};

use crate::{AppState, db};

pub const SESSION_COOKIE: &str = "ticketing_session";

pub fn hash_password(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes())
        .expect("hachage du mot de passe impossible")
        .to_string()
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

/// Jeton aléatoire hexadécimal (32 octets = 64 caractères).
pub fn random_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("générateur aléatoire indisponible");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn session_cookie(token: &str) -> String {
    format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=43200")
}

pub fn clear_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
}

pub fn cookie_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == SESSION_COOKIE)
        .map(|(_, v)| v.to_string())
}

/// Technicien connecté. Redirige vers /login sinon.
pub struct Agent(pub db::User);

/// Administrateur connecté.
pub struct Admin(pub db::User);

impl FromRequestParts<AppState> for Agent {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = cookie_token(parts).ok_or_else(|| Redirect::to("/login").into_response())?;
        let conn = state.db.lock().unwrap();
        match db::session_user(&conn, &token) {
            Ok(Some(u)) => Ok(Agent(u)),
            _ => Err(Redirect::to("/login").into_response()),
        }
    }
}

impl FromRequestParts<AppState> for Admin {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let Agent(u) = Agent::from_request_parts(parts, state).await?;
        if u.is_admin {
            Ok(Admin(u))
        } else {
            Err((axum::http::StatusCode::FORBIDDEN, "Accès réservé aux administrateurs").into_response())
        }
    }
}
