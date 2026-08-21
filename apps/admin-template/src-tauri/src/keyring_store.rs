//! OS keyring storage for the desktop auto-login credential (spec M11).
//!
//! Deliberately its own tiny module rather than inline in `lib.rs`: this is
//! the one piece of `src-tauri` that talks to `keyring` directly, and
//! keeping it a thin, uniformly-erroring wrapper makes the "keyring backend
//! unavailable" degrade path (spec M11: "keyring 不在環境（一部Linux）で機能
//! が安全に degrade する") a single place to reason about instead of
//! scattered `keyring::Error` matches through the command handlers below.
//!
//! The credential is looked up by `(service, account)` where `service` is
//! fixed for this app and `account` is the account's `username` - the same
//! convention `keyring`'s own examples use, and one `Entry` per username
//! means a future "switch which account autologs in" never collides with a
//! previously-configured one still sitting in the OS store under its own
//! username.
//!
//! ## モバイルには OS キーリングが無い（Phase 8）
//!
//! `keyring` はデスクトップの資格情報ストア（Windows Credential Manager /
//! macOS Keychain / freedesktop secret-service）専用で、Android 向けには
//! そもそもビルドが通らない —— Linux 向けの `sync-secret-service` が D-Bus を
//! 引き込むため。
//!
//! そこで `cfg(not(any(target_os = "android", target_os = "ios")))` で分ける。
//! **同じ述語が `Cargo.toml` の `keyring` の target テーブルにもある。**
//! 片方だけ直すと「依存はあるのに使えない」か「使うのに依存が無い」の
//! どちらかになるので、必ず揃えて直すこと（cfg 述語はマクロ展開されない
//! ので、1箇所にまとめる書き方ができない）。
//!
//! モバイルでは呼び出しがその場でエラーになる。自動ログインは
//! 「使えないので毎回ログインする」に degrade する —— これは keyring
//! 不在の Linux で既に通っている経路と同じで（spec M11）、新しい壊れ方を
//! 増やしていない。
//!
//! 生体認証で置き換える案はあるが、Phase 8 では追わない。持ち歩く端末の
//! 認証は据え置き機と別に考えるべきで、「PC で使っている仕組みを移植する」
//! で決めてよい話ではない。

use banto_core::BantoError;

/// Keyring service name for every credential this app stores. Not derived
/// from `CARGO_PKG_NAME` at runtime on purpose: renaming the crate later must
/// not silently orphan credentials users already saved in their OS keyring.
///
/// This is the template's shipped default; `scripts/rename.mjs` rewrites it to
/// the app's `--identifier` on the initial template→app rename so that
/// multiple Banto-derived apps on the same OS user get separate keyring
/// namespaces instead of colliding on one `(service, account)` (issue #147).
const SERVICE_NAME: &str = "dev.banto.business";

/// Turns any `keyring::Error` (backend missing, permission denied, no entry
/// found, ...) into a `BantoError::Other` with a Japanese message, so callers
/// never need to know `keyring`'s error type - this is the "safe degrade"
/// spec M11 asks for when a platform has no usable keyring backend (e.g. some
/// headless Linux setups without a secret-service provider).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn degrade(context: &str, err: keyring::Error) -> BantoError {
    BantoError::Other(format!("{context}: {err}"))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn entry(username: &str) -> Result<keyring::Entry, BantoError> {
    keyring::Entry::new(SERVICE_NAME, username)
        .map_err(|err| degrade("OSキーリングへのアクセスに失敗しました", err))
}

/// Store `password` in the OS keyring under `username`, overwriting any
/// existing entry for that username.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn set_password(username: &str, password: &str) -> Result<(), BantoError> {
    entry(username)?
        .set_password(password)
        .map_err(|err| degrade("OSキーリングへの資格情報の保存に失敗しました", err))
}

/// Retrieve the password previously stored for `username`, or an error if
/// there is none (or the backend is unavailable).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn get_password(username: &str) -> Result<String, BantoError> {
    entry(username)?
        .get_password()
        .map_err(|err| degrade("OSキーリングからの資格情報の取得に失敗しました", err))
}

/// Remove the stored credential for `username`. Idempotent-ish in intent
/// (callers here treat "already gone"/backend errors as best-effort - see
/// `autologin_disable` in `lib.rs`, which logs and proceeds rather than
/// failing the whole command on a keyring delete error).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn delete_password(username: &str) -> Result<(), BantoError> {
    entry(username)?
        .delete_credential()
        .map_err(|err| degrade("OSキーリングからの資格情報の削除に失敗しました", err))
}

/// テスト用に、OS のキーリングではなくメモリ上の実装を使わせる。
///
/// `keyring` に触れるのをこのモジュール1つに閉じ込めるためにここへ置く
/// （モジュール冒頭の主張どおりにする）。呼び出し側が `keyring::` を
/// 直接使うと、デスクトップ限定にした依存がテストから漏れ出す。
#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
pub fn install_in_memory_backend_for_tests() {
    keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
}

// --- モバイル（OS キーリングが無い） ---------------------------------------
//
// 呼び出しはその場でエラーになる。自動ログインは「使えないので毎回
// ログインする」に degrade し、呼び出し側は keyring 不在の Linux と同じ
// 経路を通る（spec M11）。

#[cfg(any(target_os = "android", target_os = "ios"))]
fn unavailable() -> BantoError {
    BantoError::Other(
        "この端末には OS キーリングがありません。自動ログインは使えません".to_string(),
    )
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn set_password(_username: &str, _password: &str) -> Result<(), BantoError> {
    Err(unavailable())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn get_password(_username: &str) -> Result<String, BantoError> {
    Err(unavailable())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn delete_password(_username: &str) -> Result<(), BantoError> {
    Err(unavailable())
}
