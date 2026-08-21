//! [`SyncPeer`] の実物 —— LAN の PC へ HTTP で話しかける。
//!
//! ## 依存を増やしていないこと（`ADR-0002` / `docs/domain/sync.md` 9節）
//!
//! `reqwest` は **`tauri` が既に引いている**（TLS 機能なし、hyper のみ）。
//! ここで直接名指ししても依存グラフにクレートは1つも増えない。9節が避けたい
//! のは「ビルドを重くする」「TLS スタックを持ち込む」ことなので、
//! `default-features = false` のまま名指しするのは意図に沿う。
//!
//! ## 平文 HTTP
//!
//! `ADR-0003`（TLS はリバースプロキシ終端）に従い、自宅 LAN 内は平文で話す。
//! Android 側は Tauri が生成するマニフェストの debug ビルドで平文が既に
//! 許可されている（`docs/android-build.md` 4.1）。
//!
//! ## 認証
//!
//! `/api/sync/*` は認証必須で、push は editor 以上（`rest/sync.rs`）。
//! 同期のたびにログインしてトークンを取り、**そのトークンはこの構造体の
//! 寿命だけ持つ**。パスワードは呼び出し側がメモリにだけ持ち、設定へは
//! 書かない（`docs/domain/sync.md` 11.9）。

use banto_core::{BantoError, FieldError};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::sync::client::SyncPeer;
use crate::sync::protocol::{
    Handshake, HandshakeRequest, Pull, PullRequest, PushRequest, PushResult,
};

/// CSRF 用の固定ヘッダ（`banto_server::csrf`）。ブラウザ以外からでも
/// 付けないと 403 になる。
const CLIENT_HEADER: (&str, &str) = ("X-Banto-Client", "banto");

/// LAN の PC。
pub struct HttpPeer {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

/// **トークンを出さない `Debug`。**
///
/// 導出すると `{:?}` した拍子に Bearer トークンがログやテストの失敗出力へ
/// 流れる。持っているかどうかだけ分かれば診断には足りる。
impl std::fmt::Debug for HttpPeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpPeer")
            .field("base_url", &self.base_url)
            .field("token", &"<非表示>")
            .finish()
    }
}

/// PC へ入るための資格情報。
///
/// **パスワードは保存しない。** 呼び出し側（`src-tauri`）がアプリの寿命だけ
/// メモリに持ち、設定に書き出さない。
pub struct PeerCredentials<'a> {
    pub username: &'a str,
    pub password: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginBody<'a> {
    username: &'a str,
    password: &'a str,
    /// 長寿命トークンは要らない。**この同期1回ぶん**しか使わないので、
    /// 既定（8時間/アイドル1時間）で足りる。取らない方が、端末を落とした
    /// ときに残る有効期限が短い。
    remember: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginReply {
    success: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

impl HttpPeer {
    /// ログインしてトークンを得る。
    ///
    /// `base_url` の末尾のスラッシュは落とす —— 設定へ手で打つ値なので
    /// `http://192.168.1.10:1421/` と入れられる。
    pub async fn connect(
        base_url: &str,
        credentials: PeerCredentials<'_>,
    ) -> Result<Self, BantoError> {
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(field_error(
                "syncPeerUrl",
                "PC のアドレスが設定されていない".to_string(),
            ));
        }
        let http = reqwest::Client::builder()
            .build()
            .map_err(|error| unreachable_error(&base_url, error))?;

        let reply: LoginReply = post(
            &http,
            &format!("{base_url}/api/auth/login"),
            None,
            &LoginBody {
                username: credentials.username,
                password: credentials.password,
                remember: false,
            },
            &base_url,
        )
        .await?;

        // ログイン失敗は 200 + `{success:false}` で返る（`auth.rs` の
        // `login_handler`）。HTTP の状態だけ見ていると素通りする。
        if !reply.success {
            let message = reply
                .error
                .unwrap_or_else(|| "PC のユーザー名かパスワードが違う".to_string());
            return Err(field_error("syncPeerPassword", message));
        }
        let token = reply
            .token
            .ok_or_else(|| BantoError::Other("PC がトークンを返さなかった".to_string()))?;

        Ok(Self {
            base_url,
            token,
            http,
        })
    }

    async fn call<B: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R, BantoError> {
        post(
            &self.http,
            &format!("{}{path}", self.base_url),
            Some(&self.token),
            body,
            &self.base_url,
        )
        .await
    }
}

impl SyncPeer for HttpPeer {
    async fn handshake(&self, request: HandshakeRequest) -> Result<Handshake, BantoError> {
        self.call("/api/sync/handshake", &request).await
    }

    async fn pull(&self, request: PullRequest) -> Result<Pull, BantoError> {
        self.call("/api/sync/pull", &request).await
    }

    async fn push(&self, request: PushRequest) -> Result<PushResult, BantoError> {
        self.call("/api/sync/push", &request).await
    }
}

/// JSON を POST して JSON を読む。
///
/// **エラー応答は PC が返した `ErrorBody` をそのまま伝える。** 「同期に
/// 失敗しました」に丸めると、番号が被っている・請求済みで凍結している
/// といった直せる理由が画面に出なくなる。
async fn post<B: Serialize, R: DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    body: &B,
    base_url: &str,
) -> Result<R, BantoError> {
    let mut request = http
        .post(url)
        .header(CLIENT_HEADER.0, CLIENT_HEADER.1)
        .json(body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|error| unreachable_error(base_url, error))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| unreachable_error(base_url, error))?;

    if !status.is_success() {
        return Err(remote_error(status, &text));
    }
    serde_json::from_str(&text)
        .map_err(|error| BantoError::Other(format!("PC の応答を読めなかった（{status}）: {error}")))
}

/// 相手が返したエラー本文（`banto_core::ErrorBody` の JSON）。
///
/// **`ErrorBody` 自体を使えない** —— あちらは `Serialize` だけを導出して
/// おり、読む側の型が無い。同梱した Banto のコードは Business 都合で
/// 書き換えない（`CLAUDE.md` 第3章）ので、こちらに読み取り専用の写しを
/// 置く。綴りは `packages/admin-core/src/errors.ts` が読んでいるものと同じ。
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RemoteError {
    NotFound { resource: String, id: String },
    Validation { field_errors: Vec<RemoteFieldError> },
    BadRequest { message: String },
    Unauthorized,
    Forbidden,
    Storage { message: String },
    Other { message: String },
}

#[derive(serde::Deserialize)]
struct RemoteFieldError {
    field: String,
    message: String,
}

/// 相手が返したエラー本文を `BantoError` へ戻す。
fn remote_error(status: reqwest::StatusCode, text: &str) -> BantoError {
    match serde_json::from_str::<RemoteError>(text) {
        Ok(RemoteError::Validation { field_errors }) => BantoError::Validation {
            field_errors: field_errors
                .into_iter()
                .map(|error| FieldError {
                    field: error.field,
                    message: error.message,
                })
                .collect(),
        },
        Ok(RemoteError::BadRequest { message }) => BantoError::BadRequest(message),
        Ok(RemoteError::Unauthorized) => BantoError::Unauthorized,
        Ok(RemoteError::Forbidden) => BantoError::Forbidden,
        Ok(RemoteError::NotFound { resource, id }) => BantoError::NotFound { resource, id },
        Ok(RemoteError::Storage { message }) => BantoError::Storage(message),
        Ok(RemoteError::Other { message }) => BantoError::Other(message),
        // 本文が `ErrorBody` でないこともある（プロキシや別のサーバに
        // 当たった場合）。状態コードだけでも出す。
        Err(_) => BantoError::Other(format!("PC が {status} を返した")),
    }
}

/// 届かない・繋がらない。**アドレスを添える** —— この画面で真っ先に
/// 疑うのは打ち間違いと、PC 側でサーバが起動していないこと。
fn unreachable_error(base_url: &str, error: reqwest::Error) -> BantoError {
    BantoError::Other(format!(
        "PC（{base_url}）に繋がらない: {error}。\
         同じ Wi-Fi にいるか、PC 側でサーバが起動しているかを確認すること"
    ))
}

fn field_error(field: &str, message: String) -> BantoError {
    BantoError::Validation {
        field_errors: vec![FieldError {
            field: field.to_string(),
            message,
        }],
    }
}
