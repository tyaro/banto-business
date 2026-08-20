//! Phase 5: 適格請求書の発行者情報と端数処理設定
//! （`docs/tax-calculation.md` 2 / `CLAUDE.md` 第8章）。
//!
//! **実値はリポジトリに書かない。** 登録番号（`T` + 13桁）・屋号・住所・
//! 振込先はここで設定データとして保持し、ドキュメント・テスト・シードには
//! サンプル値しか置かない（`CLAUDE.md` 第8章）。
//!
//! 保存先は Banto の `settings` テーブル（key/value）。専用テーブルを作らない
//! のは、Banto が既に持っている入れ物を使うため（`CLAUDE.md` 第2章）。
//! `SettingsService` の汎用 `get`/`set` 経由で読み書きし、同梱コードには
//! 手を入れない（`CLAUDE.md` 第3章）。

use crate::tax::RoundingMode;
use banto_admin_services::settings::SettingsService;
use banto_core::{BantoError, FieldError};
use serde::{Deserialize, Serialize};

const KEY_NAME: &str = "business.issuer.name";
const KEY_REGISTRATION_NUMBER: &str = "business.issuer.registration_number";
const KEY_ADDRESS: &str = "business.issuer.address";
const KEY_BANK_ACCOUNT: &str = "business.issuer.bank_account";
const KEY_ROUNDING_MODE: &str = "business.invoice.rounding_mode";

const MAX_NAME_LEN: usize = 120;
const MAX_ADDRESS_LEN: usize = 200;
const MAX_BANK_ACCOUNT_LEN: usize = 200;

/// 発行者情報。未設定の項目は `None`（確定時にそのまま NULL で焼き付ける）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuerSettings {
    /// 屋号・氏名。
    pub name: Option<String>,
    /// 適格請求書発行事業者の登録番号（`T` + 13桁）。
    pub registration_number: Option<String>,
    pub address: Option<String>,
    /// 振込先。請求書PDFに印字するだけで、Invoice にはスナップショットしない
    /// （`schema.md` §4.1 に列が無いため。口座変更は過去分の再印刷にも及ぶ）。
    pub bank_account: Option<String>,
    /// 消費税額の端数処理方向（`tax-calculation.md` 4.3）。既定は切捨て。
    /// **確定時に Invoice へスナップショットする**ので、ここを変えても
    /// 発行済みの請求書の税額は変わらない。
    pub rounding_mode: RoundingMode,
}

/// 更新ペイロード。`rounding_mode` はコード文字列（`FLOOR` / `ROUND` / `CEIL`）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuerInput {
    pub name: Option<String>,
    pub registration_number: Option<String>,
    pub address: Option<String>,
    pub bank_account: Option<String>,
    pub rounding_mode: String,
}

/// 登録番号の形式（`T` + 13桁の数字）。国税庁の公表形式で、桁数と接頭辞だけを
/// 検査する（チェックディジットの検証はしない — 誤入力の大半は桁数で捕まる）。
fn validate_registration_number(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('T')) && value.len() == 14 && chars.all(|c| c.is_ascii_digit())
}

fn trimmed(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn check_len(errors: &mut Vec<FieldError>, field: &str, value: &Option<String>, max: usize) {
    if let Some(v) = value {
        if v.chars().count() > max {
            errors.push(FieldError {
                field: field.to_string(),
                message: format!("{max}文字以内で入力してください"),
            });
        }
    }
}

/// 発行者情報のサービス層（conventions §2）。
#[derive(Clone)]
pub struct IssuerService {
    settings: SettingsService,
}

impl IssuerService {
    pub fn new(settings: SettingsService) -> Self {
        Self { settings }
    }

    /// 未設定と「空欄で保存した」を同じ `None` に畳む。`settings` は値の削除を
    /// 持たないため、空欄は空文字として保存される（[`IssuerService::set`]）。
    async fn text(&self, key: &str) -> Result<Option<String>, BantoError> {
        Ok(self.settings.get(key).await?.filter(|v| !v.is_empty()))
    }

    pub async fn get(&self) -> Result<IssuerSettings, BantoError> {
        let rounding_mode = self
            .settings
            .get(KEY_ROUNDING_MODE)
            .await?
            .and_then(|v| RoundingMode::from_code(&v))
            // 未設定・未知のコードは既定（切捨て）に倒す。設定が壊れていても
            // 請求が止まらないようにするため（値は確定時にスナップショット
            // されるので、後から直しても発行済みには影響しない）。
            .unwrap_or_default();
        Ok(IssuerSettings {
            name: self.text(KEY_NAME).await?,
            registration_number: self.text(KEY_REGISTRATION_NUMBER).await?,
            address: self.text(KEY_ADDRESS).await?,
            bank_account: self.text(KEY_BANK_ACCOUNT).await?,
            rounding_mode,
        })
    }

    pub async fn set(&self, input: IssuerInput) -> Result<IssuerSettings, BantoError> {
        let name = trimmed(&input.name);
        let registration_number = trimmed(&input.registration_number);
        let address = trimmed(&input.address);
        let bank_account = trimmed(&input.bank_account);

        let mut errors = Vec::new();
        check_len(&mut errors, "name", &name, MAX_NAME_LEN);
        check_len(&mut errors, "address", &address, MAX_ADDRESS_LEN);
        check_len(
            &mut errors,
            "bankAccount",
            &bank_account,
            MAX_BANK_ACCOUNT_LEN,
        );
        if let Some(number) = &registration_number {
            if !validate_registration_number(number) {
                errors.push(FieldError {
                    field: "registrationNumber".to_string(),
                    message: "登録番号は T で始まる13桁の数字で入力してください".to_string(),
                });
            }
        }
        let rounding_mode = match RoundingMode::from_code(input.rounding_mode.trim()) {
            Some(mode) => mode,
            None => {
                errors.push(FieldError {
                    field: "roundingMode".to_string(),
                    message: "端数処理方向が不正です".to_string(),
                });
                RoundingMode::default()
            }
        };
        if !errors.is_empty() {
            return Err(BantoError::Validation {
                field_errors: errors,
            });
        }

        // 空欄は空文字で保存する（`settings` にキーを消す API が無いため）。
        // 読み出し側の `text` が空文字を `None` に畳むので、「一度入れて
        // 消した」と「最初から未設定」が同じ扱いになる。
        for (key, value) in [
            (KEY_NAME, name.as_deref()),
            (KEY_REGISTRATION_NUMBER, registration_number.as_deref()),
            (KEY_ADDRESS, address.as_deref()),
            (KEY_BANK_ACCOUNT, bank_account.as_deref()),
        ] {
            self.settings.set(key, value.unwrap_or("")).await?;
        }
        self.settings
            .set(KEY_ROUNDING_MODE, rounding_mode.as_code())
            .await?;

        self.get().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate_memory;

    async fn service() -> IssuerService {
        let pool = migrate_memory().await.expect("migrate_memory");
        IssuerService::new(SettingsService::new(pool))
    }

    fn input() -> IssuerInput {
        IssuerInput {
            // 架空の値（CLAUDE.md 第8章：実値はリポジトリに書かない）。
            name: Some("架空設計事務所".to_string()),
            registration_number: Some("T1234567890123".to_string()),
            address: Some("架空県架空市1-2-3".to_string()),
            bank_account: Some("架空銀行 架空支店 普通 1234567".to_string()),
            rounding_mode: "FLOOR".to_string(),
        }
    }

    #[tokio::test]
    async fn defaults_to_floor_when_unset() {
        let service = service().await;
        let settings = service.get().await.expect("get");
        assert_eq!(settings.rounding_mode, RoundingMode::Floor);
        assert_eq!(settings.name, None);
    }

    #[tokio::test]
    async fn round_trips_issuer_settings() {
        let service = service().await;
        let saved = service.set(input()).await.expect("set");
        assert_eq!(saved.name.as_deref(), Some("架空設計事務所"));
        assert_eq!(saved.registration_number.as_deref(), Some("T1234567890123"));
        assert_eq!(saved.rounding_mode, RoundingMode::Floor);
        let reloaded = service.get().await.expect("get");
        assert_eq!(reloaded, saved);
    }

    #[tokio::test]
    async fn rejects_a_malformed_registration_number() {
        let service = service().await;
        let mut bad = input();
        bad.registration_number = Some("1234567890123".to_string());
        let err = service.set(bad).await.unwrap_err();
        match err {
            BantoError::Validation { field_errors } => {
                assert_eq!(field_errors[0].field, "registrationNumber");
            }
            other => panic!("unexpected: {other:?}"),
        }

        let mut short = input();
        short.registration_number = Some("T123".to_string());
        assert!(service.set(short).await.is_err());
    }

    #[tokio::test]
    async fn rejects_an_unknown_rounding_mode() {
        let service = service().await;
        let mut bad = input();
        bad.rounding_mode = "NEAREST".to_string();
        let err = service.set(bad).await.unwrap_err();
        assert!(matches!(err, BantoError::Validation { .. }), "{err:?}");
    }

    /// 空欄は `None` として読み戻る（空文字が画面に出ない）。
    #[tokio::test]
    async fn blank_fields_read_back_as_none() {
        let service = service().await;
        let mut blank = input();
        blank.name = Some("   ".to_string());
        blank.registration_number = None;
        let saved = service.set(blank).await.expect("set");
        assert_eq!(saved.name, None);
        assert_eq!(saved.registration_number, None);
    }
}
