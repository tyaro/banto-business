# Android ビルド（Phase 8 ステップ5）

Pixel 10 Fold に入れて使うための手順。**ストアを経由せず、自分の端末へ
APK を直接入れる**（`docs/domain/sync.md` 8節）。

> **どこまで確かめてあるか。** このリポジトリの CI と開発コンテナには
> Android SDK / NDK / Rust の Android ターゲットが無い（8節の決定：
> 「Android ビルドを CI に足さない」）。
>
> | 章 | 状態 |
> | --- | --- |
> | 2. アプリ側の対応 | **検証済**（依存グラフ・単体テスト） |
> | 3〜5. 実機ビルドと端末設定 | **未検証**（手順のみ） |
> | 6. PC 2台での予行 | **検証済**（実際に走らせた） |
>
> 実機で最初に通したときに、詰まった箇所をこのファイルへ追記すること。

---

## 1. なぜ Tauri のまま Android へ持っていくのか

PWA + IndexedDB を採らないのは、**金額ロジックを TypeScript で二重実装する
ことになる**ため（8節）。`conventions §2` によりサービス層が `tauri` /
`axum` に依存していないので、税計算・採算計算・消込ロジックの Rust コードが
そのまま乗る。`CLAUDE.md` 第6章の「金額が絡むロジックは必ずテストする」を
2箇所で担保する羽目にならない。

---

## 2. アプリ側の対応（済）

### 2.1 `keyring` をデスクトップ限定にした

OS キーリング（自動ログイン、spec M11）は Android には無い。Linux 用の
`sync-secret-service` が D-Bus を引き込むため、**依存に入っているだけで
Android 向けのビルドが通らない**。

`src-tauri/Cargo.toml` の target テーブルと `src/keyring_store.rs` の
`#[cfg]` で、`cfg(not(any(target_os = "android", target_os = "ios")))` に
限定した。cfg 述語はマクロ展開されないので**同じ述語が2箇所にある**。
片方だけ直すと「依存はあるのに使えない」か「使うのに依存が無い」になる。

確認方法（NDK 不要）:

```sh
cd apps/admin-template/src-tauri
cargo tree -p admin-template --target x86_64-unknown-linux-gnu -i keyring  # 出る
cargo tree -p admin-template --target aarch64-linux-android   -i keyring  # 出ない
```

モバイルでは呼び出しがその場でエラーになり、自動ログインは「使えないので
毎回ログインする」に degrade する。keyring 不在の Linux で既に通っている
経路と同じで、新しい壊れ方は増えていない。

生体認証で置き換える案はあるが Phase 8 では追わない。持ち歩く端末の認証は
据え置き機と別に考えるべきで、「PC の仕組みを移植する」で決めてよい話では
ない。

### 2.2 `window-vibrancy` は元から Windows 限定

`cfg(target_os = "windows")` の target テーブルにあり、`vibrancy_*` コマンドは
それ以外で「非対応」に degrade する。Android 向けに触るところは無い。

### 2.3 起動時に採番レンジを当てるようにした

**これが無いと Android ビルドは通っても使えない。**

`sync.device.id` を 1 にしただけでは採番は変わらない。SQLite の
`AUTOINCREMENT` は `sqlite_sequence` の値から続きを振るので、起動時に
レンジ先頭まで進めておく必要がある（`docs/domain/sync.md` 3.4）。

`sync::apply_device_range_at_startup` を、Tauri アプリと `banto-serve` の
両方の起動経路に入れた。**設定が読めないときは起動しない** —— 「0 として
続ける」を選ぶと、番号を書き損じたスマホが PC のレンジで採番を始め、同期
して初めて衝突に気付く。その時点では両方に行が入っており機械的に直せない。
止まれば設定を直すだけで復帰できる。

---

## 3. 前提（未検証）


| 要るもの | 備考 |
| --- | --- |
| JDK 17 以上 | Android Gradle Plugin の要求 |
| Android SDK（Platform 34+ / Build-Tools / Platform-Tools） | Android Studio か `cmdline-tools` から |
| Android NDK | `libsqlite3-sys` が SQLite の C ソースを同梱ビルドするため必須 |
| 環境変数 `ANDROID_HOME` / `NDK_HOME` | Tauri の Android コマンドが参照する |
| Rust の Android ターゲット | 下記 |

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi \
                  i686-linux-android x86_64-linux-android
```

Pixel 10 Fold は arm64 なので、実機だけなら `aarch64-linux-android` で足りる。
エミュレータも使うなら `x86_64-linux-android` を足す。

---

## 4. 初期化とビルド（未検証）

```sh
pnpm --filter admin-template build            # フロントエンドを先に作る
cd apps/admin-template/src-tauri
pnpm exec tauri android init                  # gen/android を生成（初回だけ）
pnpm exec tauri android build --apk --target aarch64
```

`gen/android` は生成物だが、`AndroidManifest.xml` に手を入れることになる
（下記 4.1）ので**コミットする**。生成し直すと手入れが消えるため、
`tauri android init` は初回だけにする。

### 4.1 ネットワーク権限（未検証）

同期は自宅 LAN 内の PC へ **平文 HTTP** で話しかける（`ADR-0003`：TLS は
リバースプロキシ終端。LAN 内なので rustls を足さない、8節/9節）。
Android 9 以降は平文通信が既定で禁止されているので、`AndroidManifest.xml`
に以下が要る見込み。

- `<uses-permission android:name="android.permission.INTERNET" />`
- 平文の許可。**`usesCleartextTraffic="true"` で全開にしない。**
  `network_security_config.xml` で **PC のアドレスだけ**を例外にする

インターネットに何も公開しない構成（前提）なので、全開にする理由が無い。

### 4.2 `embed-ui` は要らない

`embed-ui` は「LAN のブラウザへ実物の画面を配る」ための機能で、Android 版が
自分の webview に表示する画面は `frontendDist` から入る。**スマホ側で
`banto-serve` 相当を動かす必要は無い**（話しかけるのは常にスマホ側、
`docs/domain/sync.md` 11.1）。

---

## 5. 端末に入れたあとの設定（未検証）

### 5.1 デバイス番号を、データを1行も作る前に設定する

**順番を間違えると取り返しがつかない。** 番号を設定する前に工数を1件でも
入れると、その行は PC のレンジ（1〜）の id を持つ。あとから番号を変えても
既存行の id は変わらないので、PC 側の別の行と衝突する。

現状**専用の設定画面がまだ無い**ので、既存の汎用設定コマンドで入れる。

```js
// アプリの webview から（管理者でログインした状態で）
await window.__TAURI__.core.invoke('settings_set', {
	key: 'sync.device.id',
	value: '1'
});
```

設定したらアプリを再起動する。起動時に `apply_device_range_at_startup` が
`sqlite_sequence` を 1,000,000,000 まで進める。

| 端末 | 番号 | id の範囲 |
| --- | --- | --- |
| PC | 0（既定・設定不要） | 1 〜 999,999,999 |
| Pixel | 1 | 1,000,000,000 〜 1,999,999,999 |

確認は、1件作って id が 10 億台になっているかを見るのが早い（6章の予行で
実際にそうなることを確かめてある）。

> **残作業。** これは設定画面から入れられるべき。あわせて「行が既に在る
> ときは番号を変えさせない」ガードも要る（今は汎用の `settings_set` なので
> 素通りする）。

### 5.2 同期の相手（PC）のアドレス

未実装。同期を起動する画面と一緒に作る（`docs/domain/sync.md` 11.8）。

---

## 6. PC 2台で先に試す（検証済）

実機を待たずに、**`banto-serve` を2つ動かして**同期を確かめられる。
スマホ役の DB に番号 1 を入れておけばよい。

```sh
# PC 役（デバイス 0）
BANTO_DB=./pc.sqlite3 PORT=8721 cargo run -p admin-template-core --bin banto-serve

# スマホ役（デバイス 1）— 先に設定を入れてから起動する
sqlite3 ./phone.sqlite3 "INSERT OR REPLACE INTO settings (key, value) VALUES ('sync.device.id', '1');"
BANTO_DB=./phone.sqlite3 PORT=8722 cargo run -p admin-template-core --bin banto-serve
```

起動時に採番レンジが表示される。

```
banto-serve: sync device 1 (ids 1000000000..=1999999999)
```

`settings` テーブルはマイグレーション適用後にしか無いので、**一度起動して
から**設定を入れて再起動する。

再起動後、スマホ役で最初に作った行が 10 億台の id を持つことまで確認済み。

```
$ curl ... -X POST http://127.0.0.1:8722/api/customers -d '{"code":"P001",...}'
{"id": 1000000000, "code": "P001", ...}
```
