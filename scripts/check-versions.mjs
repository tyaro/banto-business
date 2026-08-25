#!/usr/bin/env node
/**
 * バージョン整合検査（CR-7。maintainability-review-2026-07.md §7 追補）。
 * 依存を足さない文化（conventions §3）に従い Node 標準ライブラリのみ。
 *
 * 背景: `v0.1.1` タグが存在する一方でマニフェストは `0.1.0` のまま、というドリフトが
 * 観測された（ChatGPT 計画レビュー #5）。これを機械で防ぐ。
 *
 * 2モード（オーナー指定で「通常CI」と「タグCI」を分離）:
 *   - 既定（通常CI）: 各グループ内でマニフェストの version が相互に一致すること。
 *   - `--tag <name>`（タグCI・リリースワークフロー）: 上記に加え、タグ名 `vX.Y.Z` が
 *     アプリ（admin-template）系 version と一致すること。
 *
 * 2グループ（2026-08-25、Android アルファ 0.1.0 仕切り直しで分離。CLAUDE.md 第3章）:
 *   - app: `apps/admin-template` 系（package.json / tauri.conf.json / core・
 *     src-tauri の Cargo.toml）。Business アプリ自体のリリースバージョン。
 *   - banto: `packages/*` と workspace `Cargo.toml` の `[workspace.package] version`。
 *     こちらは Banto フレームワーク側の値で、`docs/template-origin.md` が
 *     「派生元の値をそのまま維持し、上流に追随する」と明記している —— アプリの
 *     リリースバージョンとは別軸なので、以前のように両者を1つの一致グループに
 *     混ぜてはいけない（Business が 0.1.0 に仕切り直しても Banto 側の 1.2.0 は
 *     動かさない。CLAUDE.md 第3章「同梱した Banto のコードを書き換えない」）。
 *
 * `apps/admin-template/core` と `apps/admin-template/src-tauri` は、上記の
 * 理由により `version.workspace = true` を使わず各 Cargo.toml に直接
 * `version = "..."` を書いている（各ファイルのコメント参照）。
 *
 * 例外: ルート `package.json` は private・非配布（`"version": "0.0.0"`）のため
 * 対象から除外する（オーナー指定）。
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = (rel) => fs.readFileSync(path.join(repoRoot, rel), 'utf8');
const exists = (rel) => fs.existsSync(path.join(repoRoot, rel));

const jsonVersion = (rel) => JSON.parse(read(rel)).version;
// crate 自身の [package] version を読む（workspace 継承ではなく直接指定のもの）。
const cargoPackageVersion = (rel) => {
	const m = read(rel).match(/\[package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/);
	return m ? m[1] : undefined;
};

// --- app グループ: apps/admin-template 系（Business アプリのリリースバージョン） ---
const appSources = [
	{
		rel: 'apps/admin-template/package.json',
		version: jsonVersion('apps/admin-template/package.json')
	},
	{
		rel: 'apps/admin-template/src-tauri/tauri.conf.json',
		version: jsonVersion('apps/admin-template/src-tauri/tauri.conf.json')
	},
	{
		rel: 'apps/admin-template/core/Cargo.toml',
		version: cargoPackageVersion('apps/admin-template/core/Cargo.toml')
	},
	{
		rel: 'apps/admin-template/src-tauri/Cargo.toml',
		version: cargoPackageVersion('apps/admin-template/src-tauri/Cargo.toml')
	}
].filter((s) => s.version);

// --- banto グループ: 同梱 Banto パッケージ + workspace Cargo.toml の派生元バージョン ---
const bantoSources = fs
	.readdirSync(path.join(repoRoot, 'packages'))
	.map((d) => `packages/${d}/package.json`)
	.filter(exists)
	.map((rel) => ({ rel, version: jsonVersion(rel) }))
	.filter((s) => s.version);
const workspaceCargoM = read('Cargo.toml').match(
	/\[workspace\.package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/
);
if (workspaceCargoM) bantoSources.push({ rel: 'Cargo.toml', version: workspaceCargoM[1] });

let failures = 0;
const fail = (msg) => {
	console.error(`  ✗ ${msg}`);
	failures++;
};

const checkGroup = (name, sources) => {
	const versions = [...new Set(sources.map((s) => s.version))];
	if (versions.length > 1) {
		fail(`[${name}] グループ内で version が不一致:`);
		for (const s of sources) console.error(`      ${s.rel}: ${s.version}`);
	}
	return sources[0]?.version;
};

const appVersion = checkGroup('app', appSources);
checkGroup('banto', bantoSources);

const tagIdx = process.argv.indexOf('--tag');
if (tagIdx !== -1) {
	const tag = process.argv[tagIdx + 1] ?? '';
	const expected = `v${appVersion}`;
	// プレリリースタグ（v0.1.0-alpha.1 等）を許す。マニフェスト側は基底
	// バージョンのみ持つ運用（docs/android-build.md 8.5）なので、
	// 「v<version>」そのもの、または「v<version>-<プレリリース>」を一致とする。
	const matches = tag === expected || tag.startsWith(`${expected}-`);
	if (!matches)
		fail(
			`タグ名 "${tag}" が app グループの version と不一致（期待 "${expected}" または "${expected}-<プレリリース>"）`
		);
}

if (failures > 0) {
	console.error(`\n${failures} 件の不一致。マニフェストとタグ運用を整合させてください（CR-7）。`);
	process.exit(1);
}
console.log(
	`✔ バージョン整合: app グループ ${appSources.length} マニフェストが ${appVersion} で一致 / ` +
		`banto グループ ${bantoSources.length} マニフェストが ${bantoSources[0]?.version} で一致` +
		` (ルート package.json は private=0.0.0 で例外)` +
		(tagIdx !== -1 ? ` / タグ ${process.argv[tagIdx + 1]} 一致` : '')
);
