# Android ビルド（Phase 8 ステップ5）

Pixel 10 Fold に入れて使うための手順。**ストアを経由せず、自分の端末へ
APK を直接入れる**（`docs/domain/sync.md` 8節）。

> **APK は CI で作れる。** `.github/workflows/android-build.yml` を
> 手動起動（Actions タブ → Android build → Run workflow）すると、
> デバッグ署名の APK が成果物として出る。手元に Android SDK / NDK を
> 用意しなくてよいので、**3〜4章を自分でやる必要は無い**。
>
> 以下は「手元でビルドしたい場合」と「CI が壊れたときに何を見るか」の
> ための手順。開発コンテナには Android SDK / NDK / Rust の Android
> ターゲットが無いため、章ごとに検証状況が違う。
>
> | 章 | 状態 |
> | --- | --- |
> | 2. アプリ側の対応 | **検証済**（依存グラフ・単体テスト） |
> | 3. 手元ビルドの前提 | **未検証**（CI は不要なので通っていない経路） |
> | 4. 初期化とビルドの手順 | **検証済**（CI が同じ2コマンドで APK を出した） |
> | 4.1 ネットワーク権限 | **未着手**（まだ何もしていない） |
> | 5. 端末設定 | **未検証**（手順のみ） |
> | 6. PC 2台での予行 | **検証済**（実際に走らせた） |
> | 7. CI ビルド | **検証済**（62 MB の APK を 6分33秒で出力） |
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

### 2.3 `run()` にモバイル用のエントリポイントを付けた

デスクトップは `main.rs` の `fn main()` が `run()` を呼ぶ。Android は
`NativeActivity` が**共有ライブラリのシンボルを直接呼ぶ**ので `main` を
通らない。`#[cfg_attr(mobile, tauri::mobile_entry_point)]` がそのシンボルを
生やす。

**付け忘れの出方が悪い。** `cargo build` は通り `.so` も出るのに、APK の
組み立て段で落ちる。

```
failed to validate library: ... does not include required runtime symbols.
This means you are likely missing the tauri::mobile_entry_point macro usage
```

Rust 側は何も間違っていないので原因が見えにくい。**CI で APK を組むように
して初めて見つかった** —— `cargo check --target aarch64-linux-android` まで
では出ない（リンクではなく組み立て時の検証で落ちるため）。

### 2.4 シェルがセーフエリアを避けるようにした

Android は edge-to-edge で描画するので、ステータスバー・ナビゲーションバー・
カメラの切り欠きの下にも中身が入る。避けていないと、**左上のハンバーガー
メニューがステータスバーの時計と重なって押せない**（実機で確認）。
**修正後に Pixel 10 Fold で解消を確認済み**（2026-08-21）—— Tauri の
Android WebView が実際に非ゼロの inset を返すことも、これで裏が取れた。

- `app.html` の viewport に `viewport-fit=cover`。**これが無いと
  `env(safe-area-inset-*)` は常に 0** を返すので、CSS だけ直しても効かない
- `app.css` の `:root` で `--app-safe-{top,right,bottom,left}` を定義し、
  `Header` / `Sidebar` / `main` が参照する

Sidebar は**常設表示のときも**避ける必要がある —— 開いた Fold の内側は
タブレット寸法で、サイドバーが畳まれずに画面の左端から始まるため。

デスクトップのブラウザは inset が 0 なので表示は変わらない（計測で確認）。

### 2.5 起動時に採番レンジを当てるようにした

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

## 3. 手元でビルドする場合の前提（未検証）

**CI で作るならこの章は不要。** 7章へ飛べる。ここに書いてあるのは、手元で
ビルドしたい場合に自分で用意するものであり、CI では ubuntu ランナーが
最初から持っている（＝この手順自体は誰も通していない）。

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

## 4. 初期化とビルド（コマンドは検証済）

```sh
cd apps/admin-template
pnpm tauri android init                            # gen/android を生成（初回だけ）
pnpm tauri android build --debug --target aarch64 --apk
```

CI（7章）がこの2コマンドで APK を出しているので、**コマンド自体は通ることが
分かっている**。フロントエンドは `tauri.conf.json` の `beforeBuildCommand`
（`pnpm build`）が自動で作るので、先に手で叩く必要は無い。

`gen/android` は生成物で `.gitignore` 済み。CI は毎回作り直している。
**手を入れる予定は無い** —— 唯一の候補だったネットワーク権限が、生成物の
ままで足りることが分かったため（下記 4.1）。

### 4.1 ネットワーク権限（対応不要と判明）

同期は自宅 LAN 内の PC へ **平文 HTTP** で話しかける（`ADR-0003`：TLS は
リバースプロキシ終端。LAN 内なので rustls を足さない、8節/9節）。
Android 9 以降は平文通信が既定で禁止されているので、`AndroidManifest.xml`
へ手を入れる必要があると見込んでいた。**その見込みは外れていた。**

Tauri v2 が生成するマニフェストとビルドスクリプトが、既に両方を持っている
（`@tauri-apps/cli` 2.11.4 で確認）。

| 要るもの | 生成物の状態 |
| --- | --- |
| `<uses-permission android:name="android.permission.INTERNET" />` | マニフェストのテンプレートに最初から入っている |
| 平文の許可 | `android:usesCleartextTraffic="${usesCleartextTraffic}"`。Gradle の manifestPlaceholder で、**debug ビルドは `"true"`**、release は `"false"` |

配布しているのは**デバッグ署名の APK**（8節：ストアを経由せず自分の端末へ
直接入れる）なので、平文 HTTP はそのまま通る。

したがって `network_security_config.xml` で PC のアドレスだけを例外にする
案も**作らない**。理由は2つ。

1. release ビルドを作らない以上、絞る対象が無い
2. **絞れない。** 相手のアドレスは設定画面で入れる実行時の値
   （`sync.peer.url`）で、DHCP で変わりうる。ビルド時に固定する
   `network_security_config.xml` では表現できない

`gen/android` をコミットするかワークフローでパッチを当てるかの判断も、
これで不要になった（生成物のまま使う）。

### 4.2 `embed-ui` は要らない

`embed-ui` は「LAN のブラウザへ実物の画面を配る」ための機能で、Android 版が
自分の webview に表示する画面は `frontendDist` から入る。**スマホ側で
`banto-serve` 相当を動かす必要は無い**（話しかけるのは常にスマホ側、
`docs/domain/sync.md` 11.1）。

---

## 5. 端末に入れたあとの設定

`/sync`（サイドバーの「同期」、admin 限定）で行う。**専用のコマンドを
webview から叩く必要は無くなった。**

### 5.1 デバイス番号を、データを1行も作る前に設定する

**順番を間違えると取り返しがつかない。** 番号を設定する前に工数を1件でも
入れると、その行は PC のレンジ（1〜）の id を持つ。あとから番号を変えても
既存行の id は変わらないので、PC 側の別の行と衝突する。

| 端末 | 番号 | id の範囲 |
| --- | --- | --- |
| PC | 0（既定・設定不要） | 1 〜 999,999,999 |
| Pixel | 1 | 1,000,000,000 〜 1,999,999,999 |

**画面がこれを守らせる。** 採番レンジを持つ5テーブルに行が1つでもあると
（墓石も数える）、番号の入力欄が締まり、理由が出る
（`sync::set_device_id_before_any_rows`）。汎用の `settings_set` は素通し
なので、そちらでは入れないこと。

保存した時点で採番レンジが当たる（再起動を待たなくてよい）。確認は、
1件作って id が 10 億台になっているかを見るのが早い。

### 5.2 同期の相手（PC）

同じ画面で、PC のアドレスとユーザー名を設定する。

- アドレス … `http://192.168.1.10:1421` の形。PC 側の設定画面で LAN
  アクセスを有効にしたときに出るアドレス
- ユーザー名 … PC 側のアカウント。push は editor 以上が要る
- パスワード … **保存しない。** 同期の画面で入力し、アプリを終了するまで
  メモリに残る（`docs/domain/sync.md` 11.9）

設定できたら「今すぐ同期」を押す。結果（受け取り件数 / 送信件数 / 衝突
件数）がその場に出る。

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

---

## 7. CI でビルドする

`.github/workflows/android-build.yml`。

| 起動 | いつ |
| --- | --- |
| 手動（`workflow_dispatch`） | 手元に APK が欲しいとき |
| 月次（`schedule`） | 環境ドリフトで壊れたことに、APK が欲しくなった当日ではなく事前に気付くため |
| `pull_request` | **このワークフロー自身**を変更した PR だけ |

3つ目は、他に誰も踏まない経路だから。直した内容が正しいかを確かめる手段が
これしか無い。

実測（初回、キャッシュ無し）:

| ステップ | 時間 |
| --- | --- |
| ツールチェーン準備（pnpm / Node / Rust / NDK 検出） | 23秒 |
| `pnpm install` | 2秒 |
| `tauri android init` | 9秒 |
| **APK ビルド（Rust cross-compile + Gradle）** | **5分35秒** |
| 成果物アップロード | 8秒 |
| **合計** | **6分33秒** |

APK は約 62 MB（zip 圧縮後）。成果物の保持は14日。

### 7.1 なぜ「CI に足さない」から変えたか

`docs/domain/sync.md` 8節では「SDK / NDK / JDK が要り重い」として足さない
判断をしていたが、**前提の方が間違っていた**。GitHub ホストの ubuntu
ランナーにはこの3つが最初から入っており、こちらで用意するのは Rust の
Android ターゲットだけ。

「重い」の残り半分（Rust の cross-compile と Gradle でビルド時間が伸びる）も、
実測 6分半で当初の想定よりずっと軽かった。それでも毎 PR で回すほどではない
ので、**手動起動と月次に限る**。

### 7.2 署名しない

出るのは**デバッグ署名の APK**。ストアを経由せず自分の端末へ直接入れる前提
（8節）なので、リリース署名の鍵を CI に置く理由が無い。**secrets を1つも
使わない**ので、`CLAUDE.md` 第8章の「fork からの PR では渡らない前提で
設計する」も自明に満たす。

Android は未署名の APK を入れられないが、デバッグ署名で足りる。同じ鍵で
署名し続ける限り上書き更新もできる（鍵が変わると一度アンインストールが
要る）。

### 7.3 端末への入れ方

Actions の run ページから `banto-business-debug-apk` を落とし、端末へ
転送して開く。「提供元不明のアプリ」の許可が要る。

`adb` があるなら PC から直接入れられる。

```sh
adb install -r banto-business.apk
```

### 7.4 `gen/android` は毎回作り直している

`tauri android init` の生成物で `.gitignore` 済みなので、CI では毎回
生成している。**手を入れる予定は無い**（4.1）—— 唯一の候補だった
ネットワーク権限は、生成されるマニフェストのままで足りる。手を入れる必要が
生じたら、「コミットする」か「ワークフローでパッチを当てる」かを先に
決めること。今のまま手で直しても次の run で消える。
