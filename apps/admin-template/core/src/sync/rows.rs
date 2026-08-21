//! Phase 8: 同期で運ぶ「行」の表現（`docs/domain/sync.md` 11節）。
//! conventions §2 に従い `tauri` / `axum` / RBAC を知らない。
//!
//! ## なぜテーブルごとの型を使わないのか
//!
//! `Customer` / `WorkLog` 等のドメイン型はそのまま同期に使えない。
//!
//! - 表示のための結合列を持つ（`WorkLog` の `project_name` 等）。同期に載せると
//!   相手側で「どの列が実体でどれが導出か」を分けて書く羽目になる
//! - **`deleted_at` を持たない。** 墓石は同期の主役なので落とせない
//! - 8テーブルぶんの直列化と UPSERT を書くと、列を1本足したときに直す場所が
//!   8×3 箇所になる
//!
//! 代わりに **列の目録（[`SYNCED_TABLES`]）** を1箇所に持ち、SQL を目録から
//! 組み立てる。目録が実際のスキーマとずれていないことは
//! [`tests::the_manifest_matches_the_database`] が DB 自身に問い合わせて
//! 検査する —— 0024 で outbox の記録をトリガに寄せたのと同じ考え方で、
//! 「足したのに書き忘れた」が CI で落ちるようにする。
//!
//! ## 列の型が2種で足りる理由
//!
//! 同期対象8テーブルの列は **TEXT か INTEGER しかない**。
//!
//! - 金額は必ず INTEGER（円）で、`REAL` は使わない（`CLAUDE.md` 1.1）
//! - 日付は業務日付の TEXT（`CLAUDE.md` 第4章）
//! - 真偽値は INTEGER の 0/1
//! - BLOB は無い（領収書は `attachments`、DB とは別チャネル）
//!
//! PostgreSQL 側は整数列がすべて `BIGINT` なので、`i64` 一本で両方言を
//! 読める（`INTEGER`=int4 が混ざると sqlx の型検査で落ちる）。目録検査は
//! 列名だけでなく**この型の対応も**見る。

use banto_core::{BantoError, FieldError};
use banto_storage::Db;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeMap;

/// 同期対象の列に現れる型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ColumnKind {
    /// SQLite `INTEGER` / PostgreSQL `BIGINT`。
    Int,
    /// 両方言とも `TEXT`。
    Text,
}

/// 1テーブルの同期上の姿。
pub struct TableSpec {
    pub name: &'static str,
    /// 主キー列。整数 PK の5テーブルは `id`、コード PK の3テーブルは
    /// それぞれのコード列（`docs/domain/sync.md` 3.0）。
    pub key: &'static str,
    pub key_kind: ColumnKind,
    /// **主キーと `deleted_at` を含む全列。** 部分集合にしない —— 落ちた列は
    /// 相手側で既定値のまま残り、金額であれば静かに数字が食い違う。
    pub columns: &'static [(&'static str, ColumnKind)],
}

impl TableSpec {
    /// `SELECT` に並べる列名（宣言順）。
    pub fn column_list(&self) -> String {
        self.columns
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn kind_of(&self, column: &str) -> Option<ColumnKind> {
        self.columns
            .iter()
            .find(|(name, _)| *name == column)
            .map(|(_, kind)| *kind)
    }
}

use ColumnKind::{Int, Text};

/// 双方向同期の対象8テーブル（`docs/domain/sync.md` 1.1）。
///
/// **並びは依存順**（親が先）。同期を受け取った側はこの順に流し込めば
/// 外部キーで弾かれない。逆順に並べ替えたくなったら、受け側の適用順も
/// 一緒に考えること。
pub const SYNCED_TABLES: [TableSpec; 8] = [
    TableSpec {
        name: "work_categories",
        key: "code",
        key_kind: Text,
        columns: &[
            ("code", Text),
            ("name", Text),
            ("excluded_from_effective_rate", Int),
            ("sort_order", Int),
            ("active", Int),
            ("deleted_at", Text),
        ],
    },
    TableSpec {
        name: "expense_categories",
        key: "code",
        key_kind: Text,
        columns: &[
            ("code", Text),
            ("name", Text),
            ("default_tax_category", Text),
            ("sort_order", Int),
            ("active", Int),
            ("deleted_at", Text),
        ],
    },
    // `work_categories` の後（`work_category_code` が指す先）。
    TableSpec {
        name: "cost_rates",
        key: "work_category_code",
        key_kind: Text,
        columns: &[
            ("work_category_code", Text),
            ("hourly_rate", Int),
            ("updated_at", Text),
            ("deleted_at", Text),
        ],
    },
    TableSpec {
        name: "customers",
        key: "id",
        key_kind: Int,
        columns: &[
            ("id", Int),
            ("code", Text),
            ("name", Text),
            ("contact_person", Text),
            ("address", Text),
            ("phone", Text),
            ("email", Text),
            ("billing_name", Text),
            ("closing_day", Int),
            ("payment_month_offset", Int),
            ("payment_day", Int),
            ("note", Text),
            ("created_at", Text),
            ("updated_at", Text),
            ("deleted_at", Text),
        ],
    },
    TableSpec {
        name: "projects",
        key: "id",
        key_kind: Int,
        columns: &[
            ("id", Int),
            ("code", Text),
            ("customer_id", Int),
            ("name", Text),
            ("status", Text),
            ("started_on", Text),
            ("due_on", Text),
            ("estimate_amount", Int),
            ("contract_amount", Int),
            ("scope", Text),
            ("note", Text),
            ("created_at", Text),
            ("updated_at", Text),
            // 0018 で後から足した列。目録検査があるので追従漏れは CI で落ちる。
            ("billing_hourly_rate", Int),
            ("deleted_at", Text),
        ],
    },
    // `work_logs` / `expenses` より先（`trip_id` が指す先）。
    TableSpec {
        name: "trips",
        key: "id",
        key_kind: Int,
        columns: &[
            ("id", Int),
            ("project_id", Int),
            ("destination", Text),
            ("start_on", Text),
            ("end_on", Text),
            ("onsite_days", Int),
            ("nights", Int),
            ("note", Text),
            ("created_at", Text),
            ("updated_at", Text),
            ("deleted_at", Text),
        ],
    },
    TableSpec {
        name: "work_logs",
        key: "id",
        key_kind: Int,
        columns: &[
            ("id", Int),
            ("project_id", Int),
            ("trip_id", Int),
            ("worked_on", Text),
            ("work_category_code", Text),
            ("minutes", Int),
            // 記録時点で焼き付けた原価単価（`CLAUDE.md` 1.2）。**同期でレート
            // マスタが後から届いても、この値は上書きしない。**
            ("applied_rate", Int),
            ("internal_cost", Int),
            ("description", Text),
            ("invoiced", Int),
            ("created_at", Text),
            ("updated_at", Text),
            ("deleted_at", Text),
        ],
    },
    TableSpec {
        name: "expenses",
        key: "id",
        key_kind: Int,
        columns: &[
            ("id", Int),
            ("project_id", Int),
            ("trip_id", Int),
            ("spent_on", Text),
            ("expense_category_code", Text),
            ("payee", Text),
            ("amount", Int),
            ("tax_category", Text),
            ("description", Text),
            ("billable", Int),
            ("invoiced", Int),
            ("created_at", Text),
            ("updated_at", Text),
            ("deleted_at", Text),
        ],
    },
];

/// テーブル名 → 目録。同期対象外の名前には `None`。
pub fn table_spec(name: &str) -> Option<&'static TableSpec> {
    SYNCED_TABLES.iter().find(|spec| spec.name == name)
}

/// 1つの列の値。
///
/// `#[serde(untagged)]` なので、素の JSON の `null` / 数値 / 文字列として
/// 出入りする。包み紙を付けない（`{"Int":3}` にしない）のは、相手側が
/// Rust とは限らない将来も見据えて、行が素直な JSON オブジェクトに
/// 見えるようにするため。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SyncValue {
    /// NULL。`untagged` の候補は上から順に試されるので、**必ず先頭**に置く。
    Null,
    Int(i64),
    Text(String),
}

/// 同期で運ぶ1行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRow {
    /// [`SYNCED_TABLES`] のテーブル名。
    pub table: String,
    /// outbox の `row_key` と同じ形（整数 PK は10進文字列）。
    pub key: String,
    /// **列名は DB の綴りのまま（スネークケース）**。ここだけ他の API と
    /// 綴りが違うのは意図的で、camelCase へ寄せると列名の変換表が要る。
    /// 変換表は列を足したときに片側だけ直る形の事故が起きるうえ、
    /// `deleted_at` のような同期の要が黙って落ちても気付けない。
    pub values: BTreeMap<String, SyncValue>,
}

impl SyncRow {
    pub fn get(&self, column: &str) -> Option<&SyncValue> {
        self.values.get(column)
    }

    /// 墓石か（`deleted_at` が入っているか）。
    pub fn is_deleted(&self) -> bool {
        !matches!(self.get("deleted_at"), None | Some(SyncValue::Null))
    }
}

/// 指定した主キーの行を**論理削除ぶんも含めて**読む。
///
/// [`crate::sync::live`] を使わないのは、墓石こそ相手に伝えたいため。
/// 削除を伝えないと、消したはずの行が相手側に残り、次の同期でこちらへ
/// 戻ってくる。
///
/// 存在しない主キーは黙って落ちる（返る行数が減る）。物理削除はしない設計
/// なので通常は起きないが、起きたとしても「送るものが無い」だけで、
/// 相手のデータを壊す方向には働かない。
pub async fn read_rows(
    db: &Db,
    spec: &TableSpec,
    keys: &[String],
) -> Result<Vec<SyncRow>, BantoError> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let dialect = db.dialect();
    let sql = format!(
        "SELECT {} FROM {} WHERE {} IN ({})",
        spec.column_list(),
        spec.name,
        spec.key,
        dialect.placeholders(keys.len())
    );

    // 整数 PK の列は両方言とも整数型なので、10進文字列のまま bind すると
    // PostgreSQL の型検査で落ちる。ここで i64 に戻す。
    let bind_ints = match spec.key_kind {
        ColumnKind::Int => Some(
            keys.iter()
                .map(|key| parse_int_key(spec, key))
                .collect::<Result<Vec<i64>, BantoError>>()?,
        ),
        ColumnKind::Text => None,
    };

    match db {
        Db::Sqlite(pool) => {
            let mut query = sqlx::query(&sql);
            for (index, key) in keys.iter().enumerate() {
                query = match &bind_ints {
                    Some(ints) => query.bind(ints[index]),
                    None => query.bind(key.clone()),
                };
            }
            let rows = query
                .fetch_all(pool)
                .await
                .map_err(banto_storage::storage_error)?;
            rows.iter().map(|row| to_sync_row(spec, row)).collect()
        }
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => {
            let mut query = sqlx::query(&sql);
            for (index, key) in keys.iter().enumerate() {
                query = match &bind_ints {
                    Some(ints) => query.bind(ints[index]),
                    None => query.bind(key.clone()),
                };
            }
            let rows = query
                .fetch_all(pool)
                .await
                .map_err(banto_storage::storage_error)?;
            rows.iter().map(|row| to_sync_row(spec, row)).collect()
        }
    }
}

/// 10進文字列の主キーを `i64` に戻す。
///
/// 読めない値はエラーにする。読み飛ばすと「その行だけ永久に同期されない」
/// が黙って起きる。
fn parse_int_key(spec: &TableSpec, key: &str) -> Result<i64, BantoError> {
    key.parse::<i64>().map_err(|_| BantoError::Validation {
        field_errors: vec![FieldError {
            field: "key".to_string(),
            message: format!("{} の主キーが整数として読めない: {key}", spec.name),
        }],
    })
}

/// 1行を目録どおりに読み出す。
///
/// 両方言の行型で同じ処理を回すためジェネリックにしてある（SQLite と
/// PostgreSQL で書き分けると、列の読み方が2箇所に分かれて片方だけ直る）。
fn to_sync_row<R>(spec: &TableSpec, row: &R) -> Result<SyncRow, BantoError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    for<'a> Option<i64>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Option<String>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
{
    let mut values = BTreeMap::new();
    for (name, kind) in spec.columns {
        let value = match kind {
            ColumnKind::Int => row
                .try_get::<Option<i64>, _>(*name)
                .map_err(banto_storage::storage_error)?
                .map_or(SyncValue::Null, SyncValue::Int),
            ColumnKind::Text => row
                .try_get::<Option<String>, _>(*name)
                .map_err(banto_storage::storage_error)?
                .map_or(SyncValue::Null, SyncValue::Text),
        };
        values.insert((*name).to_string(), value);
    }

    // 主キーは outbox の `row_key` と同じ綴りに揃える（整数は10進文字列）。
    let key = match values.get(spec.key) {
        Some(SyncValue::Int(id)) => id.to_string(),
        Some(SyncValue::Text(code)) => code.clone(),
        _ => {
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: spec.key.to_string(),
                    message: format!("{} の主キーが NULL で読めない", spec.name),
                }],
            })
        }
    };

    Ok(SyncRow {
        table: spec.name.to_string(),
        key,
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use std::collections::BTreeSet;

    fn customer_input(code: &str, name: &str) -> CustomerInput {
        CustomerInput {
            code: code.to_string(),
            name: name.to_string(),
            contact_person: None,
            address: None,
            phone: None,
            email: None,
            billing_name: None,
            closing_day: DAY_END_OF_MONTH,
            payment_month_offset: 1,
            payment_day: DAY_END_OF_MONTH,
            note: None,
        }
    }

    #[test]
    fn every_synced_table_declares_its_own_primary_key_as_a_column() {
        for spec in &SYNCED_TABLES {
            assert_eq!(
                spec.kind_of(spec.key),
                Some(spec.key_kind),
                "{} の主キー {} が列目録に無い（または型が違う）",
                spec.name,
                spec.key
            );
        }
    }

    /// 墓石を落とすと「削除したことが相手へ伝わらない」——
    /// 消したはずの行が次の同期で復活する。
    #[test]
    fn every_synced_table_carries_its_tombstone_column() {
        for spec in &SYNCED_TABLES {
            assert_eq!(
                spec.kind_of("deleted_at"),
                Some(ColumnKind::Text),
                "{} に deleted_at が無い",
                spec.name
            );
        }
    }

    /// 目録の並びは `docs/domain/sync.md` 1.1 の8テーブルと一致すること。
    #[test]
    fn the_manifest_covers_exactly_the_eight_bidirectional_tables() {
        let names: BTreeSet<&str> = SYNCED_TABLES.iter().map(|spec| spec.name).collect();
        let expected: BTreeSet<&str> = [
            "customers",
            "projects",
            "work_categories",
            "expense_categories",
            "cost_rates",
            "work_logs",
            "expenses",
            "trips",
        ]
        .into_iter()
        .collect();
        assert_eq!(names, expected);
    }

    /// 親が子より先に並んでいること（受け側がこの順に流し込める）。
    #[test]
    fn parents_come_before_their_children() {
        let order: Vec<&str> = SYNCED_TABLES.iter().map(|spec| spec.name).collect();
        let at = |name: &str| order.iter().position(|n| *n == name).expect(name);
        assert!(at("work_categories") < at("cost_rates"));
        assert!(at("customers") < at("projects"));
        assert!(at("projects") < at("trips"));
        assert!(at("trips") < at("work_logs"));
        assert!(at("trips") < at("expenses"));
        assert!(at("work_categories") < at("work_logs"));
        assert!(at("expense_categories") < at("expenses"));
    }

    /// **目録が実際のスキーマと一致していること。**
    ///
    /// 列を足したのに目録へ書き忘れると、その列は同期で運ばれず、相手側では
    /// 既定値のまま残る。金額列なら数字が静かに食い違う。DB 自身に列を
    /// 問い合わせて突き合わせ、書き忘れを CI で落とす。
    #[tokio::test]
    async fn the_manifest_matches_the_database() {
        let db = migrate_memory().await.unwrap();
        let pool = db.as_sqlite().expect("sqlite");

        for spec in &SYNCED_TABLES {
            let rows = sqlx::query_as::<_, (String, String)>(&format!(
                "SELECT name, type FROM pragma_table_info('{}')",
                spec.name
            ))
            .fetch_all(pool)
            .await
            .expect("pragma");
            assert!(!rows.is_empty(), "{} が存在しない", spec.name);

            let actual: BTreeSet<(String, ColumnKind)> = rows
                .into_iter()
                .map(|(name, sql_type)| {
                    let kind = match sql_type.as_str() {
                        "INTEGER" => ColumnKind::Int,
                        "TEXT" => ColumnKind::Text,
                        other => panic!("{}.{name} が想定外の型: {other}", spec.name),
                    };
                    (name, kind)
                })
                .collect();
            let declared: BTreeSet<(String, ColumnKind)> = spec
                .columns
                .iter()
                .map(|(name, kind)| ((*name).to_string(), *kind))
                .collect();

            assert_eq!(
                declared, actual,
                "{} の列目録がスキーマとずれている",
                spec.name
            );
        }
    }

    #[tokio::test]
    async fn a_row_round_trips_through_the_manifest() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        let created = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");

        let spec = table_spec("customers").expect("spec");
        let rows = read_rows(&db, spec, &[created.id.to_string()])
            .await
            .expect("read");
        assert_eq!(rows.len(), 1);

        let row = &rows[0];
        assert_eq!(row.table, "customers");
        assert_eq!(row.key, created.id.to_string());
        assert_eq!(row.get("id"), Some(&SyncValue::Int(created.id)));
        assert_eq!(
            row.get("name"),
            Some(&SyncValue::Text("架空商事".to_string()))
        );
        // 未設定の任意項目は NULL のまま運ばれる（空文字に化けない）。
        assert_eq!(row.get("note"), Some(&SyncValue::Null));
        assert!(!row.is_deleted());
        // 目録の全列が揃っていること。
        assert_eq!(row.values.len(), spec.columns.len());
    }

    /// 論理削除した行も読めること。読めないと削除が相手へ伝わらない。
    #[tokio::test]
    async fn a_tombstone_is_readable_and_marked_deleted() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        let created = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        customers.delete(created.id).await.expect("delete");

        let spec = table_spec("customers").expect("spec");
        let rows = read_rows(&db, spec, &[created.id.to_string()])
            .await
            .expect("read");
        assert_eq!(rows.len(), 1, "墓石も読めること");
        assert!(rows[0].is_deleted());
    }

    #[tokio::test]
    async fn a_code_keyed_table_uses_its_code_as_the_key() {
        let db = migrate_memory().await.unwrap();
        let spec = table_spec("work_categories").expect("spec");
        let rows = read_rows(&db, spec, &["DESIGN".to_string()])
            .await
            .expect("read");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "DESIGN");
        assert_eq!(
            rows[0].get("code"),
            Some(&SyncValue::Text("DESIGN".to_string()))
        );
    }

    #[tokio::test]
    async fn an_unparsable_integer_key_is_an_error_not_a_skip() {
        let db = migrate_memory().await.unwrap();
        let spec = table_spec("customers").expect("spec");
        assert!(read_rows(&db, spec, &["DESIGN".to_string()]).await.is_err());
    }

    #[tokio::test]
    async fn an_empty_key_list_reads_nothing() {
        let db = migrate_memory().await.unwrap();
        let spec = table_spec("customers").expect("spec");
        assert!(read_rows(&db, spec, &[]).await.expect("read").is_empty());
    }

    #[test]
    fn a_value_serialises_as_bare_json() {
        assert_eq!(serde_json::to_string(&SyncValue::Null).unwrap(), "null");
        assert_eq!(serde_json::to_string(&SyncValue::Int(-3)).unwrap(), "-3");
        assert_eq!(
            serde_json::to_string(&SyncValue::Text("あ".to_string())).unwrap(),
            "\"あ\""
        );
        assert_eq!(
            serde_json::from_str::<SyncValue>("null").unwrap(),
            SyncValue::Null
        );
        assert_eq!(
            serde_json::from_str::<SyncValue>("42").unwrap(),
            SyncValue::Int(42)
        );
        assert_eq!(
            serde_json::from_str::<SyncValue>("\"x\"").unwrap(),
            SyncValue::Text("x".to_string())
        );
    }

    #[test]
    fn unknown_tables_have_no_spec() {
        assert!(table_spec("invoices").is_none());
        assert!(table_spec("sync_outbox").is_none());
    }
}
