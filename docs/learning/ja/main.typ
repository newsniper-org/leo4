#import "../../template/leo4-book.typ": book
#show: book.with(
  title: "leo4 — 学習資料",
  subtitle: "日本語版",
  author: "윤병익 (leo4 プロジェクト)",
  lang: "ja",
)

= はじめに

`leo4` は Lean 4 と Rust の相互運用ライブラリで、意図的に Rust
側を特定の Lean ツールチェーンバージョンに *結びつけない* 設計
です。前作の `leo3` は `lean.h` に対して直接コンパイルされ、
Lean の内部レイアウトが変わるたびに壊れていました。`leo4` は
すべての Lean ABI 知識をビルド時に生成される C シムに閉じ込め、
Rust クレートには安定した正規 ABI のみを公開します。

その結果、Rust クレートは IDL (WIT の上位互換である小さな
スキーマ言語) を追跡し、Lean ツールチェーンは追跡しません。
Lean のアップグレードはシムを回転させますが、Rust バイナリには
影響しません。

この学習資料はシニアエンジニアが leo4 を学ぶ順序で進めます:
表層 (ユーザーは何を書くのか?) から始めて層を一枚ずつ剥がし
(どのように境界を越えて転送されるのか?)、最後にアーキテクチャ
を導いた設計判断を見ていきます。

== 想定読者

Lean 4 または Rust のいずれかに馴染みがあり、境界横断を追える
程度にもう一方を学ぶ意欲があることを前提とします。前提知識:

- Rust の基礎: `Cargo.toml`、trait、ライフタイム (`'a`)、
  procedural macro のユーザーレベル理解 (自分で書く必要はなく、
  生成されるコードの意味が分かれば十分)。
- Lean 4 の基礎: `def`、`structure`、`inductive`、型クラス
  (`class` / `instance`)、Lean の式が抽象型とコンパイルされた
  ランタイム表現の両方を持つという考え方。
- C ABI レベルの外部関数インターフェース (FFI) の漠然とした
  感覚 --- ポインタ、sizeof、呼び出し規約。

wasm Component Model や WASIp3 は、それぞれの章を読むときに
だけ必要です。

= 30 秒ツアー

最も単純な leo4 のユースケース。Lean 側:

```lean
import Leo4

namespace Sample

@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b
```

Rust 側:

```rust
mod sample {
    leo4::import! {
        fn add(a: u64, b: u64) -> u64;
    }
}

fn main() -> Result<(), leo4::LeanError> {
    let lean = leo4::Lean::open(
        env!("LEO4_SHIM_SO"),
        env!("LEO4_HANDSHAKE_FILE"),
    )?;
    let r = sample::add(&lean, 2, 3)?;
    assert_eq!(r, 5);
    Ok(())
}
```

`@[leo4_export]` は Lake プラグインに「この定義は境界を跨ぐ」
と伝えます。Rust 側の `leo4::import!` はプラグインが生成した
mangling テーブルを読み、引数を leo4 の正規 ABI でエンコードし、
対応する C シム入口を呼び出し、戻り値をデコードして `Result` で
包む Rust ラッパーを合成します。

= アーキテクチャ概観

leo4 は六つの可動部品で構成されます。各々の責務を知れば、
メンタルモデルの半分は出来上がります。

== Lake プラグイン (`lake/Leo4Plugin/`)

ユーザーパッケージをロードし、すべての `@[leo4_export]` 定義
を巡り、ビルドごとに四つの成果物を生成する Lean 実行ファイル:

#table(
  columns: (auto, 1fr),
  table.header[*ファイル*][*目的*],
  [`<pkg>.leo4-schema`],
  [正規 IDL 形式: 型宣言 + 関数シグネチャを安定したテキスト
   形式で。スキーマハッシュの入力。],
  [`<pkg>.leo4-mangling`],
  [論理関数名 + 引数型 mangling から、シムが呼び出す一意の C
   シンボルへのマッピング JSON テーブル。],
  [`<pkg>.leo4-handshake`],
  [スキーマハッシュ + Lean ツールチェーン識別子 + エクスポート
   インターフェースのリスト。Rust ローダが `Lean::open` 時点
   で読み込む。],
  [`<pkg>.leo4-shim.{c,so}`],
  [生成された C ソースを共有ライブラリにコンパイル。
   エクスポート毎に一つの `leo4_call_<mangled>` 入口。
   システムで唯一 `lean/lean.h` を `#include` する場所。],
)

プラグインは `<pkg>.leo4-exports.lean` も書き出します ---
シムがリンクする Lean ラッパーモジュールで、ユーザーの
エクスポートを既知名 surface に包む
`@[export leo4_lean__<mangled>]` 宣言を提供します。

== `leo4-abi` (正規 ABI marshalling)

`lake/Leo4/Leo4/Marshal.lean` と `Builtins.lean` をバイト単位
で mirror する Rust クレート。両側が `LeanMarshal` trait /
型クラスを実装し、テストスイート (`tests/conformance/`) が
サポートされる全型について Lean エンコーダと Rust エンコーダ
がバイト単位で同一の出力を生成することを検証します。

== `leo4-mslean4` (loader + dispatch)

`Lean::open`、`Arena<'a>`、`LeanRef<'a, T>` を提供する Rust
クレート。ローダは `libloading` でシムの `.so` を取り込み、
プロセスごとに一度 Lean ランタイムを初期化し、スキーマ
ハッシュを Rust 内蔵定数と照合し、ラッパーモジュールの
`initialize_*` シンボルを実行した後、名前ごとの関数ポインタ
キャッシュ経由で `leo4_call_<mangled>` 呼び出しをディスパッチ
します。

== `leo4-macros` (`leo4::import!`, `#[derive(LeanMarshal)]`)

Procedural macro。`leo4::import!` は `fn` シグネチャの extern
スタイルブロックをパースし、ビルドスクリプトが `OUT_DIR` 経由
で surface する mangling JSON から検索して Rust ラッパー関数を
emit します。`#[derive(LeanMarshal)]` は四つの正規 ABI shape
(record、all-unit enum、mixed-payload variant、single-`u64`
resource) に当てはまるユーザー型の encode/decode を合成します。

== `leo4` ファサード

薄い re-export クレート。ユーザーは一行追加するだけ:
`leo4 = { workspace = true }`。それ以外のすべて --- `Lean`、
`LeanRef`、`LeanError`、`import!`、`LeanMarshal` --- は
`leo4::*` にあります。

== `leo4-build`

`build.rs` ヘルパー。消費者クレートの `build.rs` 一行:

```rust
fn main() {
    leo4_build::wire("path/to/<pkg>/.lake/build/leo4").unwrap();
}
```

これがマクロとローダが期待する `cargo:rustc-link-search`、
`cargo:rerun-if-changed=`、`env!("LEO4_SHIM_SO")` /
`env!("LEO4_HANDSHAKE_FILE")` 定数を emit します。

= IDL --- WIT の上位互換

leo4 の IDL は両側間の正式な型レベルインターフェースです。
WebAssembly Component Model の WIT から始めて、Lean の依存型
が境界に収まるために必要な小さな構成要素を追加しました。

文法は `SPEC/idl-grammar.ebnf` にあります。WIT に対する主な
拡張:

#table(
  columns: (auto, 1fr),
  table.header[*構成要素*][*理由*],
  [nominal 宣言の `generic_params`],
  [Lean のユーザー定義型はジェネリック。`record Pair<α, β>` は
   二つの型パラメータを持つ record としてパースされ、各
   インスタンス化が独自の mangled name を得る。],
  [`Self` / `Self<…>` 自己参照],
  [`Tree { leaf, node(Self, Self) }` のような variant は外側の
   宣言を通じて再帰する。Mangling 規則
   (`SPEC/mangling.md` §"Self and Self<…>") は完全 FQN ではなく
   短いトークンを emit する。],
  [`mutual { … }` クラスタ + `Cyc<i>`],
  [Phase 6: 二つの nominal 型間の相互再帰。`Cyc<i>` はクラスタ
   の `i` 番目のメンバを参照する。],
  [`constraint <name> = <body>` 宣言],
  [`oneof { … }` のような制約がジェネリックの admit-set を
   固定する。型レベル専用; ワイヤに到達しない。],
  [`bigint` / `bignat`],
  [任意精度整数。ワイヤ形式は符号 + 桁
   (SPEC/canonical-abi.md §6)。],
  [`external <fqn>`],
  [Phase 8: フィールド毎 codegen の代わりにカスタム
   `LeanMarshal` インスタンスにワイヤ形式が宿る nominal 型。
   proof-carrying フィールドを持つ `Rat` などの
   Mathlib-shape 型に使用。],
)

WIT 側では `leo4c lower` が各 IDL フラグメントを WIT
ファイルに下します。WIT 出力は `wasm-tools` と
`wit-bindgen` で Component Model 配備に向けて消費できます。

= 正規 ABI --- ワイヤ上のバイト

`SPEC/canonical-abi.md` が規範です。Rust と Lean の
エンコーダは同じ論理値について同じバイトを生成する必要があり、
conformance harness (`tests/conformance/run.sh`) が 29 個の
fixture で固定しています。

スペック全体を読みたくない場合の要点:

- 整数はリトルエンディアン; 符号付きと符号無しは同じビット
  パターンを共有 (符号付きは 2 の補数)。
- 文字列は `u32 len + utf-8 バイト`。
- リストは `u32 len + N 要素エンコーディング`。
- Option は `u8 disc (0=none, 1=some) + payload`。
- Result は `u8 disc (0=ok, 1=err) + payload`。
- Variant は `u32 LE disc + payload` (SPEC §9; 2026-05-20 の
  commit b2aa323 で u32 を確定 --- SPEC は ≤256 ケースに対して
  u8 を許可しているが両エンコーダとも 4 バイト emit)。
- Record はフィールドエンコーディングを宣言順に連結。
- Resource は不透明な `u64` ハンドル。
- `bigint` / `bignat` は長さ前置の limb 配列 + 符号。

シムエミッタと Rust derive マクロがこの形式に従うコードを生成
します。プラグインの `walkUserDecl` がユーザー型を発見し、
型毎の手書きなしで対応する encode/decode を合成します。

= Mangling --- 命名規則

`SPEC/mangling.md` が C シンボル名を定義します。完全形は:

```
leo4__<pkg_seg>__<iface>__<fname>__<arg_mangles>__h<schema_hash>
```

各片は ASCII 安全; FQN のドットはアンダースコアになる; generic
引数はインスタンス化毎セグメントに展開。スキーマハッシュは
正規化された IDL テキストに対する `FNV-1a-64` で、13 文字
base32lc でレンダリング。あるエクスポートのシグネチャが
変わるとハッシュが回転し、したがってパッケージのすべての
mangled name が回転します --- なので fresh シムとリンクする
古い Rust バイナリはリンク時に失敗します。

ハッシュ構築は `SPEC/mangling.md` §3 にドキュメント化されて
います。二つの実装 (Rust の `crates/schema-idl` と Lean の
`lake/Leo4Plugin/Leo4Plugin/Mangling.lean`) は同一に計算する
必要があり、`tests/mangling/` が両側 67+ 名をバイト単位で同一
に固定します。

= Rust 側の型システム

境界は二つの主な trait を使います:

- `LeanMarshal` --- 正規 ABI encode/decode。すべての primitive
  型、composite (`Vec<T>`、`Option<T>`、`Result<T,E>`、tuple)
  に実装、`#[derive]` を通じてユーザー record、enum、variant、
  resource にも。
- `LeanType` --- スキーマ層に繋がる型システムマーカー。
  多くのユーザは直接触らない; `#[derive]` とマクロが処理する。

不透明ハンドル用の `LeanResource` もあります。型は同時に
`LeanMarshal` と `LeanResource` の両方であることはできません
--- プラグインがこれを強制します。

Lean 側は `class Leo4.LeanMarshal` と一致する `deriving
LeanMarshal` ハンドラで trait / 型クラスを mirror します。
二つのバイトストリームは一致しなければならず、conformance
harness が cross-impl チェック。

= Phase ラダー --- 各機能の着地時点

leo4 開発は phase ラダーに従います。各機能がどの phase 由来
かを知れば、コミットメッセージを読むときに役立ちます。

#table(
  columns: (auto, 1fr),
  table.header[*Phase*][*Landed*],
  [0], [Lake hook spike --- 正しいプラグイン統合点
        (`lake build` 後に呼ばれる `lean_exe`、`recBuildLean`
        フックではない) を発見。],
  [1], [Lean ランタイムライブラリ + Lake プラグイン;
        admit-set アルゴリズム。],
  [2], [Rust `leo4-idl` + cross-impl mangling 適合性。],
  [3], [WIT lowering パス + `wasm-tools` 検証。],
  [4], [正規 ABI 適合性 harness、`bignat` / `bigint`。],
  [5], [C シム合成 + `leo4-mslean4` + `leo4-macros` +
        `examples/01-hello`、`examples/02-roundtrip`。
        エンドツーエンドパイプライン。],
  [6], [nominal 型間の相互再帰 (`mutual { … }` IDL ブロック、
        `Cyc<i>`、`examples/04-mutual-ast`)。],
  [7], [非同期 `io<T>` lowering。パーサが `future<T>` /
        `stream<T>` を desugar; シムが `IO α` Lean ラッパー
        を `lean_io_result_*` で包む。wasm-async surface
        のための WASIp3 sibling プロジェクト。],
  [8], [Mathlib 互換 subset: `LeanRat`、`LeanU128` /
        `LeanI128`、`LeanComplexF{32,64}x2`、`LeanF16` /
        `LeanBF16` / `LeanF128` (nightly)、IEEE-754 RTNE
        丸めの Mathlib bridge。],
)

= 結びに

この学習資料は出発点です。連携する `implement-from-scratch`
ガイドブックが次の一歩を踏みます: leo4 の各層を自分で
*構築する* 過程を、元の phase が着地した順序で案内します。

日常参照用:

- `SPEC/*.md` は規範; 何かが不明確ならスペックを確認。
- `CHANGELOG.md` はすべてのコミットの効果と根拠を列挙。
- `ROADMAP.md` は phase ラダーを記述。
- `LEO4-DESIGN.md` はすべてのアーキテクチャ判断とその根拠を
  捉える。

リポジトリが唯一の真実の源。それ以外はすべて注釈です。
