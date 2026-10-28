//! Connexion via un fournisseur d'identité OpenID Connect
//! (Microsoft 365 / Entra ID, Google Workspace, ou tout fournisseur OIDC).
//!
//! Flux « authorization code » avec PKCE. Le jeton d'identité est reçu directement
//! du fournisseur par une connexion HTTPS côté serveur : conformément à la norme
//! OpenID Connect (§3.1.3.7), la validation TLS remplace la vérification de signature.
//! Les revendications (émetteur, audience, expiration, nonce, domaine) sont vérifiées.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, PartialEq)]
pub enum Provider {
    Microsoft,
    Google,
    Generic,
}

pub struct Oidc {
    provider: Provider,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    allowed_domain: String,
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct IdClaims {
    iss: String,
    aud: Audience,
    exp: i64,
    nonce: Option<String>,
    email: Option<String>,
    email_verified: Option<bool>,
    /// Microsoft : identifiant de connexion (UPN), en général l'adresse e-mail.
    preferred_username: Option<String>,
    name: Option<String>,
    /// Google Workspace : domaine du compte.
    hd: Option<String>,
}

/// Identité vérifiée renvoyée par le fournisseur.
pub struct Identity {
    pub email: String,
    pub name: String,
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl Oidc {
    /// Lit la configuration (variables `OIDC_*`) et interroge le fournisseur.
    /// Retourne `Ok(None)` si la connexion par fournisseur n'est pas configurée.
    pub async fn from_env(base_url: &str, allowed_domain: &str) -> Result<Option<Oidc>, String> {
        let Some(provider_name) = env("OIDC_PROVIDER") else {
            return Ok(None);
        };
        let (provider, issuer) = match provider_name.to_lowercase().as_str() {
            "microsoft" => {
                let tenant = env("OIDC_TENANT").ok_or(
                    "OIDC_TENANT est obligatoire pour Microsoft (ID du locataire ou domaine, ex. sias.fr)",
                )?;
                (Provider::Microsoft, format!("https://login.microsoftonline.com/{tenant}/v2.0"))
            }
            "google" => (Provider::Google, "https://accounts.google.com".to_string()),
            "generic" => (
                Provider::Generic,
                env("OIDC_ISSUER").ok_or("OIDC_ISSUER est obligatoire avec OIDC_PROVIDER=generic")?,
            ),
            other => return Err(format!("OIDC_PROVIDER inconnu : {other} (microsoft, google ou generic)")),
        };
        let client_id = env("OIDC_CLIENT_ID").ok_or("OIDC_CLIENT_ID manquant")?;
        let client_secret = env("OIDC_CLIENT_SECRET").ok_or("OIDC_CLIENT_SECRET manquant")?;

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| e.to_string())?;
        let url = format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'));
        let disc: Discovery = http
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("découverte OIDC impossible ({url}) : {e}"))?
            .json()
            .await
            .map_err(|e| format!("réponse de découverte OIDC invalide : {e}"))?;

        Ok(Some(Oidc {
            provider,
            client_id,
            client_secret,
            redirect_uri: format!("{base_url}/auth/callback"),
            allowed_domain: allowed_domain.to_string(),
            issuer: disc.issuer,
            authorization_endpoint: disc.authorization_endpoint,
            token_endpoint: disc.token_endpoint,
            http,
        }))
    }

    /// Libellé affiché sur le bouton de connexion.
    pub fn label(&self) -> &'static str {
        match self.provider {
            Provider::Microsoft => "Se connecter avec Microsoft 365",
            Provider::Google => "Se connecter avec Google",
            Provider::Generic => "Se connecter avec le compte de l'entreprise",
        }
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub fn authorize_url(&self, state: &str, nonce: &str, verifier: &str) -> String {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut params = vec![
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("scope", "openid email profile"),
            ("state", state),
            ("nonce", nonce),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ];
        match self.provider {
            Provider::Google => params.push(("hd", self.allowed_domain.as_str())),
            Provider::Microsoft => params.push(("domain_hint", self.allowed_domain.as_str())),
            Provider::Generic => {}
        }
        reqwest::Url::parse_with_params(&self.authorization_endpoint, &params)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| self.authorization_endpoint.clone())
    }

    /// Échange le code contre un jeton d'identité et vérifie l'identité obtenue.
    pub async fn exchange(&self, code: &str, verifier: &str, nonce: &str) -> Result<Identity, String> {
        let resp = self
            .http
            .post(&self.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .map_err(|e| format!("appel du fournisseur impossible : {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("échange du code refusé ({status}) : {body}"));
        }
        let token: TokenResponse = resp.json().await.map_err(|e| format!("réponse invalide : {e}"))?;
        self.validate(&token.id_token, nonce)
    }

    fn validate(&self, id_token: &str, nonce: &str) -> Result<Identity, String> {
        let payload = id_token.split('.').nth(1).ok_or("jeton d'identité mal formé")?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .map_err(|_| "jeton d'identité mal formé")?;
        let c: IdClaims = serde_json::from_slice(&bytes).map_err(|e| format!("jeton d'identité invalide : {e}"))?;

        if c.iss != self.issuer {
            return Err(format!("émetteur inattendu : {}", c.iss));
        }
        let aud_ok = match &c.aud {
            Audience::One(a) => *a == self.client_id,
            Audience::Many(v) => v.contains(&self.client_id),
        };
        if !aud_ok {
            return Err("jeton destiné à une autre application".into());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if c.exp < now - 60 {
            return Err("jeton expiré".into());
        }
        if c.nonce.as_deref() != Some(nonce) {
            return Err("nonce invalide".into());
        }

        let email = match self.provider {
            Provider::Microsoft => c.email.clone().or(c.preferred_username.clone()),
            _ => {
                if c.email_verified == Some(false) {
                    return Err("adresse e-mail non vérifiée par le fournisseur".into());
                }
                c.email.clone()
            }
        }
        .ok_or("le fournisseur n'a pas transmis d'adresse e-mail")?
        .trim()
        .to_lowercase();

        if self.provider == Provider::Google && !c.hd.as_deref().is_some_and(|hd| hd.eq_ignore_ascii_case(&self.allowed_domain)) {
            return Err(format!("compte Google extérieur au domaine {}", self.allowed_domain));
        }
        let domain_ok = email
            .rsplit_once('@')
            .is_some_and(|(local, d)| !local.is_empty() && d == self.allowed_domain);
        if !domain_ok {
            return Err(format!("seules les adresses @{} sont autorisées", self.allowed_domain));
        }

        let name = c.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| email.clone());
        Ok(Identity { email, name })
    }
}
