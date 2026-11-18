//! sessions: a token any serving host checks alone, bound to a key pair the
//! browser made and cannot export. the token is the session key's MAC over
//! who signed in, on which device, with which public key, until when; every
//! request also carries a fresh signature by the private key over its method,
//! path and time, so a copied token is useless without the browser that holds
//! the key.
//!
//! token: `v1.<b64url(json(claims))>.<b64url(hmac_sha256(session key, the
//! first two parts))>`. proof: `Coeus-Proof: <ms>.<b64url(r || s)>`, signed
//! over `coeus-req-v1\n<METHOD>\n<path?query>\n<ms>\n<b64url(sha256(token))>`.
//! a captured proof replays only within [WINDOW], on the same path and
//! method, which every host can check without sharing nonces.

use std::{fmt, path::Path, str::FromStr, time::Duration};

use axum::http::{HeaderMap, Method, header};
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD as B64};
use eyre::{Result, WrapErr, bail, eyre};
use hmac::{Hmac, Mac};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::{common::model::Timestamp, config::Key};

/// how far a signed moment may be from the server's clock, either way
pub const WINDOW: Duration = Duration::from_secs(120);

/// how a browser names its session in `Authorization`
const SCHEME: &str = "Coeus ";
pub const PROOF: &str = "coeus-proof";
const VERSION: &str = "v1";
const REQUEST: &str = "coeus-req-v1";
const SIGNIN: &str = "coeus-signin-v1";

/// the fleet's session keys. the first signs new sessions, and any of them
/// checks one, so a new key goes first and the old one stays until the
/// sessions it signed have expired; dropping a key ends them at once
pub struct Sessions {
    keys: Vec<SessionKey>,
    lifetime: Duration,
}

struct SessionKey {
    kid: Kid,
    key: Key,
}

/// the first bytes of a session key's sha256, naming the key a token was
/// signed with
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kid([u8; 4]);

/// what a token says, under the session key's MAC
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionClaims {
    pub v: u8,
    pub kid: Kid,
    /// the tailnet login that signed in
    pub sub: String,
    /// the tailnet node it signed in from
    pub device: String,
    /// the key every request must be signed with
    pub key: DeviceKey,
    /// when it signed in, always on the tailnet
    pub iat: Timestamp,
    pub exp: Timestamp,
}

/// a browser's P-256 public key, written as its uncompressed SEC1 point
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceKey(VerifyingKey);

/// a request's `Coeus-Proof`: when the browser signed it, and the signature
pub struct Proof {
    pub ts: Timestamp,
    pub sig: Signature,
}

/// a session [Sessions::issue] made
pub struct Session {
    pub token: String,
    pub claims: SessionClaims,
}

/// why a request is not let in. every one answers the same 401
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// no session, or no proof
    Missing,
    /// a token or proof that does not parse
    Malformed,
    /// signed with a key this host does not hold
    UnknownKey,
    /// the MAC does not match what the token says
    Forged,
    Expired,
    /// signed too far from now
    Stale,
    /// the signature is not the session key's, or not over this request
    Unproven,
}

impl Sessions {
    pub fn new(keys: Vec<Key>, days: u32) -> Result<Self> {
        if keys.is_empty() {
            bail!("no session key");
        }
        let keys = keys
            .into_iter()
            .map(|key| SessionKey {
                kid: Kid::of(&key),
                key,
            })
            .collect();
        Ok(Self {
            keys,
            lifetime: Duration::from_secs(u64::from(days) * 24 * 60 * 60),
        })
    }

    /// the keys in `path`, 64 hex digits each, one per line, the signing
    /// one first
    pub fn load(path: &Path, days: u32) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to read {}", path.display()))?;
        let keys = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(Key::from_str)
            .collect::<Result<Vec<_>>>()
            .wrap_err_with(|| format!("invalid session key in {}", path.display()))?;
        Self::new(keys, days)
    }

    /// a session for `sub` on `device`, bound to `key`, from `now`
    pub fn issue(&self, sub: String, device: String, key: DeviceKey, now: Timestamp) -> Session {
        let signer = &self.keys[0];
        let claims = SessionClaims {
            v: 1,
            kid: signer.kid,
            sub,
            device,
            key,
            iat: now,
            exp: now.after(self.lifetime),
        };
        let json = serde_json::to_vec(&claims).expect("claims always serialize");
        let signed = format!("{VERSION}.{}", B64.encode(json));
        let mac = signer.mac(signed.as_bytes()).finalize().into_bytes();
        Session {
            token: format!("{signed}.{}", B64.encode(mac)),
            claims,
        }
    }

    /// what `token` says, if one of these keys signed it and it has not
    /// expired at `now`
    pub fn check(&self, token: &str, now: Timestamp) -> Result<SessionClaims, Refused> {
        let (signed, mac) = token.rsplit_once('.').ok_or(Refused::Malformed)?;
        let (version, claims) = signed.split_once('.').ok_or(Refused::Malformed)?;
        if version != VERSION {
            return Err(Refused::Malformed);
        }
        let mac = B64.decode(mac).map_err(|_| Refused::Malformed)?;
        let claims = B64.decode(claims).map_err(|_| Refused::Malformed)?;
        let claims: SessionClaims =
            serde_json::from_slice(&claims).map_err(|_| Refused::Malformed)?;

        let key = self
            .keys
            .iter()
            .find(|key| key.kid == claims.kid)
            .ok_or(Refused::UnknownKey)?;
        key.mac(signed.as_bytes())
            .verify_slice(&mac)
            .map_err(|_| Refused::Forged)?;
        if claims.v != 1 {
            return Err(Refused::Malformed);
        }
        if now >= claims.exp {
            return Err(Refused::Expired);
        }
        Ok(claims)
    }

    /// the session a request to `path_and_query` carries, if its token
    /// checks and its proof is the session key's signature over this request,
    /// made within [WINDOW] of `now`
    pub fn authenticate(
        &self,
        method: &Method,
        path_and_query: &str,
        headers: &HeaderMap,
        now: Timestamp,
    ) -> Result<SessionClaims, Refused> {
        let token = headers
            .get(header::AUTHORIZATION)
            .ok_or(Refused::Missing)?
            .to_str()
            .map_err(|_| Refused::Malformed)?
            .strip_prefix(SCHEME)
            .ok_or(Refused::Malformed)?;
        let proof: Proof = headers
            .get(PROOF)
            .ok_or(Refused::Missing)?
            .to_str()
            .map_err(|_| Refused::Malformed)?
            .parse()
            .map_err(|_| Refused::Malformed)?;

        let claims = self.check(token, now)?;
        fresh(proof.ts, now)?;
        let message = Proof::message(method, path_and_query, proof.ts, token);
        match claims.key.verifies(&message, &proof.sig) {
            true => Ok(claims),
            false => Err(Refused::Unproven),
        }
    }
}

/// whether a moment a browser signed is within [WINDOW] of `now`
pub fn fresh(ts: Timestamp, now: Timestamp) -> Result<(), Refused> {
    match ts.0.abs_diff(now.0) <= WINDOW.as_millis() as u64 {
        true => Ok(()),
        false => Err(Refused::Stale),
    }
}

impl SessionKey {
    fn mac(&self, signed: &[u8]) -> Hmac<Sha256> {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.key.as_bytes()).expect("hmac takes any key");
        mac.update(signed);
        mac
    }
}

impl Kid {
    fn of(key: &Key) -> Self {
        let digest = Sha256::digest(key.as_bytes());
        Self([digest[0], digest[1], digest[2], digest[3]])
    }
}

impl Serialize for Kid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex(&self.0))
    }
}

impl<'de> Deserialize<'de> for Kid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let invalid = || serde::de::Error::custom("a kid is 8 hex digits");
        if text.len() != 8 {
            return Err(invalid());
        }
        let mut kid = [0; 4];
        for (at, byte) in kid.iter_mut().enumerate() {
            *byte = u8::from_str_radix(text.get(at * 2..at * 2 + 2).ok_or_else(invalid)?, 16)
                .map_err(|_| invalid())?;
        }
        Ok(Self(kid))
    }
}

impl DeviceKey {
    /// a public key as SEC1 bytes, refused unless a point on P-256
    pub fn from_sec1(bytes: &[u8]) -> Result<Self> {
        VerifyingKey::from_sec1_bytes(bytes)
            .map(Self)
            .map_err(|_| eyre!("not a P-256 public key"))
    }

    /// a public key as WebCrypto exports it: `{ kty: "EC", crv: "P-256", x, y }`
    pub fn from_jwk(jwk: &Jwk) -> Result<Self> {
        if jwk.kty != "EC" || jwk.crv != "P-256" {
            bail!("not a P-256 key");
        }
        let x = B64.decode(&jwk.x).wrap_err("x is not base64url")?;
        let y = B64.decode(&jwk.y).wrap_err("y is not base64url")?;
        if x.len() != 32 || y.len() != 32 {
            bail!("a P-256 coordinate is 32 bytes");
        }
        let mut point = vec![4];
        point.extend(x);
        point.extend(y);
        Self::from_sec1(&point)
    }

    /// whether `sig` is this key's signature over `message`
    pub fn verifies(&self, message: &[u8], sig: &Signature) -> bool {
        self.0.verify(message, sig).is_ok()
    }

    fn sec1(&self) -> Vec<u8> {
        self.0.to_encoded_point(false).as_bytes().to_vec()
    }
}

impl Serialize for DeviceKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&B64.encode(self.sec1()))
    }
}

impl<'de> Deserialize<'de> for DeviceKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = B64.decode(text).map_err(serde::de::Error::custom)?;
        Self::from_sec1(&bytes).map_err(serde::de::Error::custom)
    }
}

/// a public key as a JSON Web Key
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Jwk {
    pub kty: String,
    pub crv: String,
    pub x: String,
    pub y: String,
}

impl Proof {
    /// what a browser signs for a request
    pub fn message(method: &Method, path_and_query: &str, ts: Timestamp, token: &str) -> Vec<u8> {
        let token = B64.encode(Sha256::digest(token.as_bytes()));
        format!("{REQUEST}\n{method}\n{path_and_query}\n{}\n{token}", ts.0).into_bytes()
    }

    /// what a browser signs to sign in to the site at `origin`
    pub fn signin(origin: &str, ts: Timestamp) -> Vec<u8> {
        format!("{SIGNIN}\n{origin}\n{}", ts.0).into_bytes()
    }
}

/// a signature as WebCrypto writes it, `r || s`, in base64url
pub fn signature(text: &str) -> Result<Signature> {
    let bytes = B64.decode(text).wrap_err("not base64url")?;
    Signature::from_slice(&bytes).map_err(|_| eyre!("not a P-256 signature"))
}

impl FromStr for Proof {
    type Err = eyre::Report;

    fn from_str(value: &str) -> Result<Self> {
        let (ts, sig) = value
            .split_once('.')
            .ok_or_else(|| eyre!("a proof is <ms>.<signature>"))?;
        Ok(Self {
            ts: Timestamp(ts.parse().wrap_err("not a moment")?),
            sig: signature(sig)?,
        })
    }
}

impl fmt::Display for Proof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.ts.0, B64.encode(self.sig.to_bytes()))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
