#import "../../template/leo4-book.typ": book
#show: book.with(
  title: "leo4 — ゼロから実装するためのガイド",
  subtitle: "日本語版",
  author: "윤병익 (leo4 project)",
  lang: "ja",
)

= まえがき

本書は leo4 を白紙の状態から段階的にビルドしていく道のりを扱う。
各段階は元プロジェクトが実際にたどった順序とまったく同じである。最後まで
読み終えれば次のものが手に入る:

- `@[leo4_export]` 属性と `LeanMarshal` 型クラスを公開する Lean 4
  ライブラリ。
- ユーザパッケージの export を発見し、安定した schema hash を計算し、
  シンボルを mangle し、C シム + handshake ファイルを発行する Lake
  プラグイン。
- `libloading` でシムをロードし、呼び出しを canonical-ABI バイトに
  エンコードし、mangling テーブル経由でディスパッチし、結果をデコード
  する Rust ワークスペース。そしてそのすべてを `fn add(a: u64,
  b: u64) -> u64;` のような綺麗な宣言の背後に隠す手続きマクロ。
- 任意の拡張: WIT 変換、ジェネリックレコードのサポート、相互再帰、
  非同期 `io<T>`、Mathlib 風の carrier 型。

本書は `SPEC/*.md` の内容を複製しない。仕様は規範である。本書は仕様に
向けて構築するに過ぎない。仕様と本書が食い違うときは仕様に従う。

== 必要なもの

- `lean-toolchain` に固定されたバージョン (現行 `v4.29.1`) と一致する
  Lean 4 ツールチェーン。
- Rust ツールチェーン ≥ 1.85 (Edition 2024)。
- Lake プラグインが `leanc` 経由で利用する C コンパイラ
  (`clang` または `gcc`)。
- `cargo`, `lake`, `just`, `jq`, `wasm-tools` (任意の WIT 章で使用)。
- 集中できる数時間。パイプラインは深い。昼休みでは終わらない。

== 構成

本書は phase ladder に従う (元プロジェクトの `ROADMAP.md`)。各部は一つの
能力をエンドツーエンドで完成させる。各部の終わりにはデモを走らせて何かが
動くことを確認できる。

#table(
  columns: (auto, 1fr),
  table.header[*部*][*達成内容*],
  [I],   [Lean ランタイムライブラリと `@[leo4_export]` 属性。],
  [II],  [Lake プラグイン骨組; ジェネリック用 admit-set アルゴリズム。],
  [III], [IDL: 型、mangling、schema hash。Lean と Rust の安定契約。],
  [IV],  [Canonical-ABI マーシャル: Lean 側 (`LeanMarshal` 型クラス) と
          Rust 側 (`LeanMarshal` トレイト)。],
  [V],   [C シム発行: export ごとの `leo4_call_<mangled>` 翻訳単位。],
  [VI],  [Rust ローダ (`leo4-native`) と `leo4::import!` proc-macro。],
  [VII], [WIT 変換パスと `wasm-tools` 検証 (任意、綺麗に分離可能)。],
  [VIII],[相互再帰 + `Cyc<i>`。],
  [IX],  [非同期 `io<T>`、WASIp3 姉妹プロジェクト。],
  [X],   [Mathlib 風 carrier 型とブリッジ。],
)

どの部の終わりで止めても動くシステムが残る。第 V 部までだけでも、
スカラと文字列での Lean--Rust 往復が完成する。第 VIII 部までいけば、
本番コードが要求し得る Phase 6 表面が揃う。

= 第 I 部 --- Lean ランタイムライブラリ

まず Lean 側の表面を彫り出す。プラグインも Rust ローダも、属性、型クラス、
canonical-ABI エンコーダ/デコーダの上に乗る。それらが住む場所が
Lean ライブラリだ。

== プロジェクト構成

Lake パッケージを作る:

```
lake/
  Leo4/
    Leo4.lean          -- トップレベル re-export
    Leo4/
      Syntax.lean      -- leo4_constraint 構文カテゴリ
      Export.lean      -- @[leo4_export] 属性
      Marshal.lean     -- LeanMarshal 型クラス + LeanError
      Resource.lean    -- LeanResource マーカー
      Builtins.lean    -- プリミティブ用 LeanMarshal インスタンス
      Deriving.lean    -- deriving LeanMarshal ハンドラ
      Build.lean       -- ユーザ向けビルドヘルパ
    lakefile.lean
```

`Leo4` ライブラリは下流のすべてのパッケージが `require` する。トップ
レベルの import は最小限に抑える。

== `LeanError` の定義

Lean 側のすべての失敗可能な操作には、エラーコード + メッセージを返す
手段が必要だ。フラットな構造を使う:

```lean
namespace Leo4

structure LeanError where
  code : UInt32
  detail : String
  deriving Repr

namespace LeanError
def mk' (code : UInt32) (detail : String) : LeanError := { code, detail }
end LeanError

end Leo4
```

エラーコードは `SPEC/canonical-abi.md` §13 に従う。`0x00000001` を
`decodeError` に、`0x00000005` を handshake 不一致に、`0x00000007` を
返却バッファ不足に、`0x00000064` を unimplemented に予約する。同じ
ファイルに `def` 定数として置く。

== `@[leo4_export]` 属性

`@[leo4_export]` は空のマーカ属性。プラグインは属性拡張を問い合わせて
タグ付きの宣言を見つける。Lean 4 には `registerBuiltinAttribute` がある。

```lean
import Lean
namespace Leo4

initialize leo4ExportAttr : Lean.TagAttribute ←
  Lean.registerTagAttribute `leo4_export
    "marks a declaration as a leo4 boundary export"

end Leo4
```

健全性チェック: 別モジュールに自明な export を定義し、
`leo4ExportAttr.hasTag (← getEnv) ``YourModule.add` が `true` を返すか
確認する。

== `LeanMarshal` 型クラス

`LeanMarshal` は境界を越えるすべての値型が実装する型クラス。エンコード
側は `ByteArray` にバイトを append する。デコード側は同 ByteArray から
読み、`(value, newOffset)` をエラー経路と共に返す。

```lean
namespace Leo4

class LeanMarshal (T : Type) where
  canonicalEncode : T → ByteArray → ByteArray
  canonicalDecode : ByteArray → Nat → Except LeanError (T × Nat)

end Leo4
```

Lean の `ByteArray` は packed な `Array UInt8`。`Nat` オフセットは安全
のための無制限インデックスを与える。ワイヤ形式の長さプレフィックスが
実質的な境界を持っている。

== ビルトイン・インスタンス

スカラ型から: `UInt8`, `UInt16`, `UInt32`, `UInt64`, `Int8`...`Int64`,
`Float`, `Float32`, `Bool`, `Char`。それぞれをリトルエンディアン・
バイトとしてエンコードし、同じように読み戻す。

代表的実装、`UInt32`:

```lean
namespace Leo4

instance : LeanMarshal UInt32 where
  canonicalEncode n buf :=
    let b0 := (n.toUInt8)
    let b1 := ((n >>> 8).toUInt8)
    let b2 := ((n >>> 16).toUInt8)
    let b3 := ((n >>> 24).toUInt8)
    buf.push b0 |>.push b1 |>.push b2 |>.push b3
  canonicalDecode buf off := do
    if off + 4 > buf.size then
      throw (LeanError.mk' 1 "u32: out of bounds")
    let v : UInt32 :=
      buf[off]!.toUInt32 |||
      (buf[off+1]!.toUInt32 <<< 8) |||
      (buf[off+2]!.toUInt32 <<< 16) |||
      (buf[off+3]!.toUInt32 <<< 24)
    return (v, off + 4)

end Leo4
```

すべてのプリミティブに繰り返す。次に複合インスタンスを構築:

- `String` --- `u32 len + utf-8 bytes`。
- `List T` --- `u32 len + N elements`。
- `Option T` --- `u8 disc + payload`。
- `Except E T` --- `u8 disc + payload`。
- `α × β` --- 2 要素を連結。
- `Nat` (`bignat`) --- `u32 limb count + LE u64 limbs`。
- `Int` (`bigint`) --- `u8 sign + bignat magnitude`。

各インスタンスは双方向。デコード側の境界チェックは積極的に。ワイヤ入力は
定義上信頼できない。

== Deriving ハンドラ

Rust 側の `#[derive(LeanMarshal)]` と Lean 側の `deriving LeanMarshal`
はどちらもユーザ定義型に対してフィールド単位の encode/decode を合成する。
Lean 側はこれを `registerDerivingHandler` で行う。

```lean
namespace Leo4.Deriving

open Lean Elab Command Meta

private def mkLeanMarshalHandler (declNames : Array Name)
    : CommandElabM Bool := do
  let env ← getEnv
  for declName in declNames do
    let some (.inductInfo indVal) := env.find? declName
      | return false
    -- 形による分岐: 単一 ctor → レコード、全無引数 multi-ctor → enum、
    -- 混合 → variant。
    -- (公開プロジェクトの Deriving.lean に詳細。)
    pure ()
  return true

initialize
  registerDerivingHandler ``Leo4.LeanMarshal mkLeanMarshalHandler

end Leo4.Deriving
```

ハンドラの本体が難しい部分。帰納型の ctor を巡って encode アーム
(ctor あたり 1 match-arm、判別子を push してから各フィールドを encode)
を作り、それから decode アーム (判別子ごとに 1 arm、内側のフィールドを
取り出す) を作る。Phase 6 の mutual サポートのため、`mutual ... end`
クラスタのすべてのメンバは 1 つの `mutual ... end` ブロック内の
`partial def` encoder/decoder + メンバごとに 1 つのインスタンスにまとめる。

まずは単一形 (例: レコードのみ) の実装を着地させよう。enum / variant /
mutual のサポートは後の段階で追加できる。

== 健全性チェック

Leo4 ライブラリの外に Lean ファイルを書く:

```lean
import Leo4

structure Point where
  x : Float
  y : Float
  deriving Leo4.LeanMarshal

#eval do
  let p : Point := ⟨1.5, 2.5⟩
  let buf : ByteArray := Leo4.LeanMarshal.canonicalEncode p ByteArray.empty
  IO.println s!"encoded {buf.size} bytes: {buf.toList}"
  let (p', off) := match Leo4.LeanMarshal.canonicalDecode (T := Point) buf 0 with
    | .ok r => r | .error e => panic! s!"decode: {e.detail}"
  IO.println s!"decoded x={p'.x} y={p'.y}, ate {off} bytes"
```

16 バイト (2 つの `f64`) がエンコードされて往復することを確認。そう
ならないなら、次に進む前にエンコーダ/デコーダを直す。

= 第 II 部 --- Lake プラグイン骨組

Lake プラグインは `lake build` の後で動く `lean_exe`。ユーザパッケージの
すべての `@[leo4_export]` 定義を巡り、(ジェネリック用に) admit-set を
計算し、学習資料 3 章で挙げた成果物を発行する。

== プロジェクト構成

```
lake/
  Leo4Plugin/
    Leo4Plugin.lean      -- トップレベル
    Leo4Plugin/
      AdmitSet.lean      -- IDLType + UserDecl ADT, admit-set アルゴリズム
      Mangling.lean      -- mangleType, schema hash
      Emit.lean          -- ファイル書き出し、JSON 形状
      Main.lean          -- runPlugin ドライバ
    Main.lean            -- exe エントリーポイント
    lakefile.lean
```

`lakefile.lean` はパッケージを宣言し、`Leo4` (第 I 部のランタイム
ライブラリ) を `require` し、ルートモジュールが `Main.lean` の
`lean_exe leo4plugin` を公開する。

== Export の発見

プラグインはスタンドアロン実行ファイルとして動く。ユーザのモジュール名を
コマンドライン引数で受け取る。

```
$ lake exe leo4plugin Sample
```

エントリーは `Lean.importModules (loadExts := true)` でユーザのコンパイル
済みモジュールをロードし、環境内で `@[leo4_export]` タグ付き decl を探す:

```lean
def gatherExports (env : Environment) : Array Name := Id.run do
  let mut out : Array Name := #[]
  for (n, _) in env.constants do
    if Leo4.leo4ExportAttr.hasTag env n then
      out := out.push n
  return out
```

これがソート済みの `Name` のリストを生む。各々について、解析器は関数の
型を取り、ジェネリックパラメータ / 値パラメータ / 戻り型に分解し、
それぞれを IDL に下げる。

== IDLType ADT

プラグインの IDL 表現は Rust 側の `schema-idl::IDLType` を鏡写しにした
Lean 帰納型。`AdmitSet.lean` に置く:

```lean
inductive IDLType where
  | u8 | u16 | u32 | u64
  | i8 | i16 | i32 | i64
  | f32 | f64
  | bool | char | string
  | bigint | bignat
  | list (t : IDLType)
  | option (t : IDLType)
  | result (t : IDLType) (e : Option IDLType)
  | tuple (ts : Array IDLType)
  | record (fqn : String) (args : Array IDLType)
  | variant (fqn : String) (args : Array IDLType)
  | enumT (fqn : String)
  | flagsT (fqn : String)
  | resource (fqn : String) (args : Array IDLType)
  | io (t : IDLType)
  | self
  | selfApp (args : Array IDLType)
  | cyc (i : UInt32)
  deriving Repr, Inhabited, BEq
```

コンストラクタ 1 つ 1 つが正規 IDL。Rust 側は (Rust 命名規則を除いて)
これをそのまま鏡として持つ。mangling 規則 (第 5 章) が各々を安定 ASCII
文字列にマップする。

`UserDecl` ADT は nominal 型宣言を集める:

```lean
inductive UserDecl where
  | record   (fqn) (generics : Array Name) (fields : Array (Name × IDLType))
  | enumT    (fqn) (cases : Array Name)
  | variant  (fqn) (generics : Array Name) (cases : Array (Name × Array IDLType))
  | resource (fqn) (generics : Array Name)
  | mutual   (members : Array UserDecl)
  | externalMarshal (fqn) (generics : Array Name)
```

追加 2 コンストラクタ (`mutual`, `externalMarshal`) は第 6 段階 / 第 10
段階で合流する。ADT は最初から完全にしておく。

== Export 巡回

タグ付き `Name` ごとに `ConstantInfo` を取得し、型を telescope 分解し、
各 binder を下げる:

```lean
def analyzeExport (n : Name) : MetaM (Option ExportAnalysis) := do
  let env ← getEnv
  let some info := env.find? n | return none
  Meta.forallTelescope info.type fun args body => do
    -- binder の分類:
    --   - 暗黙 kind-typed → ジェネリック型パラメータ
    --   - 暗黙 value-typed → 消去される値パラメータ
    --   - inst-implicit → 型クラス制約
    --   - 明示的 → 境界の値パラメータ
    -- その後 exprToIDLSubst で各値パラメータ型を下げる。
    sorry
```

`exprToIDLSubst` は再帰的型下げ。Lean `Expr` と代入マップ (ジェネリック
binder → 具体 `IDLType`) を受け取り、対応する `IDLType` か、下げ不能なら
`none` を返す。`List`, `Option`, `Except`, `Prod`, `IO`, Self の短絡
特殊化。ユーザ定義帰納型は形に応じて
`record`/`variant`/`enumT`/`resource` に下る。

肝心な詳細: `Meta.forallTelescope` を (reduce なしで) 使い、元の
`IO α` の形が残るようにする。reducing 変種は `IO α = IO.RealWorld →
EStateM …` に展開され、偽の `IO.RealWorld` パラメータを露出する。

== Admit-set アルゴリズム

ジェネリックパラメータを持つ export については、プラグインが binder
制約を満たすすべての具体化を列挙する。各組み合わせに対して別々の IDL
シグネチャと mangled 名を作る。アルゴリズム:

1. 各ジェネリック `T_i` について admit-set を決める: 取りうる `IDLType`
   値の集合。デフォルト: 全プリミティブ (`unboundedAdmitSet`)。クラス
   制約あり: 各クラスの `classAdmitSet` と交差。
2. デカルト積を計算。各タプルが 1 具体化。
3. 各具体化について、パラメータ型に代入し `paramInfo` 配列を生成。

ファントム・ジェネリック (どこでも参照されない binder) は組み合わせ
爆発をスキップ --- ファントムスロットを `none` にした単一具体化のみ発行。

このアルゴリズムは公開コードベースの `Main.lean` `analyzeExport` にある。
再実装の前に一度読むとよい。エッジケース (高階ジェネリック、値ジェネリック、
Self 再帰型内のジェネリック引数) は時間を食う。

= 第 III 部 --- Mangling と schema hash

`IDLType` 値 + export リスト + 発見されたユーザ型があれば、schema hash
が入力にする安定テキスト形を作れる。

== `mangleType`

`mangleType : IDLType → String` は Lean と Rust の間でバイト同一。各
コンストラクタが固定トークンへマップ:

```
u8 → "u8"           list T  → "L_" ++ mangle T ++ "_l"
u16 → "u16"         option T → "O_" ++ mangle T ++ "_o"
...                 result T none → "Rz_" ++ mangle T ++ "__z"
i8 → "i8"           tuple [A,B] → "T_" ++ mangle A ++ "_" ++ mangle B ++ "_t"
...                 record fqn args → "S_" ++ fqnSeg fqn ++ ... ++ "_s"
                    variant fqn args → "V_" ++ ...
                    enum fqn → "E_" ++ fqnSeg fqn ++ "_e"
                    resource fqn args → "X_" ++ ...
                    Self → "self"
                    Cyc<i> → "c" ++ toString i ++ "c"
```

完全な規則は `SPEC/mangling.md` §2。

== 完全な mangled 名

```
leo4__<pkg_seg>__<iface>__<fname>__<arg_mangles>__h<schema_hash>
```

`arg_mangles` は各パラメータ型の mangle 形をアンダースコアで連結。
schema hash は *正規化された IDL 形式* (テキスト) の FNV-1a-64 を、
13 文字の base32lc (小文字、パディングなし) としてレンダリングしたもの。

FNV-1a-64 は単純。offset basis `0xCBF29CE484222325`、prime
`0x00000100000001B3`、各バイトを XOR してから乗算。Base32lc は
`abcdefghijklmnopqrstuvwxyz234567` (RFC 4648 小文字、パディングなし)。

== 正規 IDL レンダリング

`renderCanonical : Config → Array UserDecl → Array Member → Bool → String`
が次のテキストを生成:

```
package leo4-sample;
interface Sample {
  record Sample.Point { x: f64, y: f64 };
  variant Sample.Tree { leaf, node(Self, Self) };
  func add(_0: u64, _1: u64) -> u64;
  func midpoint(_0: Sample.Point, _1: Sample.Point) -> Sample.Point;
}
```

2 モード: `pretty := true` (改行、インデント) はディスク上の
`.leo4-schema` ファイル用; `pretty := false` (圧縮、トークン間単一空白)
は schema hash 入力用。

ユーザ decl はバンド内で FQN ソート (バンド 0: record/enum、バンド 1:
resource、mutual クラスタはバンド 0 だがソース順序保持)。関数は名前で
ソート。決定性は譲れない --- ハッシュはバイト同一出力に依存する。

ハッシュ入力は *圧縮* 形。UTF-8 バイトに FNV-1a を走らせて `UInt64` を
得て、ビッグエンディアンで base32 文字列に変換して接尾辞とする。

== 健全性チェック

小さなフィクスチャ (Lean 側):

```lean
@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b
```

`lake exe leo4plugin Sample` を走らせて確認:

- `tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-schema` が
  妥当なテキストファイル。
- `tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-handshake` の
  `schema_hash` JSON フィールドに 16 文字の base32lc schema hash。
- 再実行でハッシュが安定。

その後、Rust 鏡 (`crates/schema-idl/`) を並行実装し、Lean 出力を
`leo4c mangle <schema>` の出力と比較する cross-impl ハーネス
(`tests/mangling/`) を追加。両者はバイト単位で一致する必要がある。

= 第 IV 部 --- Canonical-ABI マーシャル

Lean ライブラリは `LeanMarshal` 型クラスを持つ。Rust 側にも対応する
トレイトが必要。両者は共有するすべての値型について同じバイトを生成
しなければならない。

== Rust トレイト

```rust
pub trait LeanMarshal: Sized + 'static {
    fn canonical_encode(&self, buf: &mut Vec<u8>);
    fn canonical_decode(buf: &[u8], off: usize)
        -> Result<(Self, usize), LeanError>;
}
```

エンコードは `Vec<u8>` (必要に応じて成長)、デコードは `&[u8] + off`
(Lean 側と同形)。`LeanError` は `u32` コード + `String` 詳細を運び、
Lean の `Leo4.LeanError` と一致する。

== プリミティブ impl

各 Rust プリミティブに直接 impl を書く:

```rust
impl LeanMarshal for u32 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize)
        -> Result<(Self, usize), LeanError>
    {
        if buf.len() < off + 4 {
            return Err(LeanError::new(
                error_codes::DECODE_ERROR,
                "u32: out of bounds",
            ));
        }
        let v = u32::from_le_bytes(
            buf[off..off + 4].try_into().unwrap(),
        );
        Ok((v, off + 4))
    }
}
```

すべてのプリミティブに繰り返す。正確な LE 振る舞いが重要 --- Lean の
`(n.toUInt8, ..., (n >>> 24).toUInt8)` 連鎖が生むバイトと一致する必要
がある。

== Conformance ハーネス

両側はバイト単位で一致しなければならない。フィクスチャを構築:

```
tests/conformance/
  fixtures/
    u32.lean       -- Leo4.LeanMarshal で `u32 42` をバイト出力
    u32.rs         -- leo4-abi で `42u32` をバイト出力
    point.lean     -- レコード例
    point.rs       -- 同じ
    ...
  run.sh
```

`run.sh` は同じ論理値で両フィクスチャを走らせ、バイト出力を比較し、
ペアに食い違いがあれば失敗する。出荷前に微妙なバイト順ミスを捕まえる
テストだ。

型ごとに最低 1 フィクスチャを着地: すべてのプリミティブ、すべての複合形
(list, option, result, tuple)、最低 2 つのユーザ定義型 (record, variant)。

= 第 V 部 --- C シム発行

C シムは Lean の native ABI (`lean_object*`, `lean_alloc_ctor`,
`lean_io_result_*`, …) が canonical ABI のバイトストリームと出会う場所。
プラグインはパッケージごとに 1 つの `.c` ファイルを生成し、export ×
具体化ごとに 1 つの `LEO4_EXPORT int32_t leo4_call_<mangled>(...)`
エントリを置く。

== シムソース構造

```c
#include <lean/lean.h>
#include <stdint.h>
#include <stddef.h>

#define leo4_memcpy __builtin_memcpy
#define LEO4_EXPORT __attribute__((visibility("default")))

#define LEO4_OK                          0
#define LEO4_ERR_DECODE                  0x00000001
#define LEO4_ERR_HANDSHAKE_MISMATCH      0x00000005
#define LEO4_ERR_RETURN_BUF_TOO_SMALL    0x00000007
#define LEO4_ERR_IO_FAILED               0x00010001
#define LEO4_ERR_UNIMPLEMENTED           0x00000064

typedef void leo4_arena_t;

/* ヘルパごとの extern 宣言 */
extern uint64_t leo4_lean__leo4__sample__Sample__add__u64_u64__h<hash>(uint64_t, uint64_t);

LEO4_EXPORT int32_t leo4_call_leo4__sample__Sample__add__u64_u64__h<hash>(
    leo4_arena_t* arena,
    const uint8_t* args_ptr, size_t args_len,
    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)
{
    /* 引数デコード */
    /* 呼び出し */
    /* 戻り値エンコード */
}
```

シグネチャは固定 (`SPEC/canonical-abi.md` §14)。ローダは dlsym で
バインドする。マクロはここを呼ぶ Rust コードを生成する。

== 型ごとのハンドラ

シム発行器の中核データ構造は `TyHandler`:

```lean
private structure TyHandler where
  cType        : String   -- 例: "uint64_t"
  externCType  : String   -- extern 宣言の C 型
  ownsRef      : Bool     -- 末尾で lean_dec が必要か?
  scalarKind   : Option String  -- ctor accessor 用に "uint8" 等
  ctorScalarSz : Nat
  decodeBlock  : String → String → String  -- (var, cleanup) → C
  encodeBlock  : String → String → String
  boxExpr      : String → String  -- value → lean_object*
  unboxExpr    : String → String  -- lean_object* → value
```

各 IDL 型について発行器が `TyHandler` を解決する。スカラ型は汎用
`scalarHandler` を使う。文字列は `stringHandler` (ランタイムヘルパに委譲)。
List / option / result / tuple は高階 --- `listHandler ih` は内側型の
ハンドラを受け取って包む。

ユーザ定義レコードはフィールドハンドラから `recordHandler` を生成。
variant は独自発行器が (fqn, args) 具体化ごとに 2 つのヘルパ関数
(例: `leo4_dec_Sample_Tree` / `leo4_enc_Sample_Tree`) を作り、各々が
disc + ペイロードを扱う。

variant 内の Self 参照は同じヘルパを再帰呼び出し。mutual クラスタは
`Cyc<i>` 参照を使い、発行時に peer のヘルパへ解決する (第 VIII 部)。

== メイン・レンダーループ

```lean
def renderOneShim (cfg userDecls a schemaHash params ret) : String :=
  let mangled := mangle cfg.pkg cfg.iface a.fname (params.map ...) schemaHash
  let entry  := s!"leo4_call_{mangled}"
  let helper := s!"leo4_lean__{mangled}"
  -- params から handlerFor で paramHs : Array TyHandler を構築。
  -- ret から retH    : TyHandler を構築。
  -- ハンドラのいずれかが `none` なら LEO4_ERR_UNIMPLEMENTED スタブを発行。
  -- それ以外は decode → invoke → encode の本体全体を発行。
  ...
```

export ごとに約 30--100 行の生成 C。結果を `leanc` (Lean の
include/ライブラリ・パスがプリ設定された clang) でコンパイルして `.so`
を作る。

== 健全性チェック

`lake exe leo4plugin Sample` 後、`<pkg>.leo4-shim.c` を確認する。
スカラ `add(u64, u64) -> u64` は次のように見えるはずだ:

```c
LEO4_EXPORT int32_t leo4_call_leo4__sample__Sample__add__u64_u64__h<hash>(
    leo4_arena_t* arena,
    const uint8_t* args_ptr, size_t args_len,
    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)
{
    (void)arena;
    size_t off = 0;
    uint64_t a0;
    if (args_len - off < 8u) { *ret_len = 0; return LEO4_ERR_DECODE; }
    leo4_memcpy(&a0, args_ptr + off, 8);
    off += 8u;
    uint64_t a1;
    if (args_len - off < 8u) { *ret_len = 0; return LEO4_ERR_DECODE; }
    leo4_memcpy(&a1, args_ptr + off, 8);
    off += 8u;
    if (off != args_len) { *ret_len = 0; return LEO4_ERR_DECODE; }
    uint64_t r = leo4_lean__leo4__sample__Sample__add__u64_u64__h<hash>(a0, a1);
    size_t out_off = 0;
    if (ret_cap - out_off < 8u) { *ret_len = out_off + 8u; return LEO4_ERR_RETURN_BUF_TOO_SMALL; }
    leo4_memcpy(ret_ptr + out_off, &r, 8);
    out_off += 8u;
    *ret_len = out_off;
    return LEO4_OK;
}
```

この `.c` + Lean ラッパモジュールの `.c` に対して `leanc` (または
適切なフラグの `cc`) を走らせて `<pkg>.leo4-shim.so` を作る。ユーザ
パッケージの `.so` (lake build の `precompileModules` から出るもの) を
RPATH でリンクしてラッパがユーザのコンパイル済み export を呼べる
ようにする。

= 第 VI 部 --- Rust ローダと `import!` マクロ

シム `.so`、handshake ファイル、mangling テーブルがある。Rust 側が
これらをバインドする番だ。

== `leo4-native` --- ローダ

`crates/leo4-native/` が `Lean::open` を公開:

```rust
pub struct Lean { /* libloading::Library + メタ */ }

impl Lean {
    pub fn open(
        so_path: impl AsRef<Path>,
        handshake_path: impl AsRef<Path>,
    ) -> Result<Self, LeanError> {
        // 1. handshake JSON を読む; schema_hash + wrapper_init_symbol 抽出。
        // 2. schema_hash を Rust 側定数と検証。
        // 3. libloading で `Library::new(so_path)`。
        // 4. プロセスごとに 1 回 Lean ランタイム初期化
        //    (`lean_initialize_runtime_module`、その後ラッパの
        //    `initialize_<X>` シンボル)。
        // 5. mangled シンボルごとに関数ポインタをキャッシュ。
        ...
    }

    pub fn call_shim(
        &self,
        mangled_body: &str,
        args: &[u8],
        ret: &mut [u8],
    ) -> Result<usize, LeanError> {
        // dlsym で `leo4_call_<mangled>` を引く (キャッシュ)。
        // (arena=NULL, args_ptr, args_len, ret_ptr, ret_cap, &ret_len)
        // で呼び出し。int32_t ステータスを Result に変換。
        ...
    }
}
```

ランタイム init は `std::sync::Once` でプロセスごとに 1 回。ラッパ
モジュールの `initialize_<X>` シンボルは `lean_io_result_is_ok` 方式の
成功を返す。ユーザ呼び出しのディスパッチ前にチェック。

== `leo4-macros-backend` --- マクロ展開器

`leo4::import!` は関数型手続きマクロ (`#[proc_macro]`)。extern ブロック
ライクな入力をパースし、ビルド時の mangling JSON で各 `fn` を引いて
Rust ラッパを発行:

```rust
pub fn add(lean: &Lean, a: u64, b: u64) -> Result<u64, LeanError> {
    let mut args = Vec::<u8>::with_capacity(16);
    <u64 as LeanMarshal>::canonical_encode(&a, &mut args);
    <u64 as LeanMarshal>::canonical_encode(&b, &mut args);
    let mut ret = [0u8; 8];
    let mut ret_len = 0;
    lean.call_shim(MANGLED_BODY, &args, &mut ret)?;
    let (v, _) = <u64 as LeanMarshal>::canonical_decode(&ret, 0)?;
    Ok(v)
}
```

マクロの仕事は正しい `MANGLED_BODY` を選ぶこと。`leo4-build` が設定する
`LEO4_MANGLING_FILE` を読み、関数名 + 各引数の IDL 形 (`rust_type_to_idl`
で計算) でマッチする。複数具体化のジェネリック export では、
`#[leo4(args = "u64,str")]` 属性でユーザが明示選択できる。

== `leo4-build` --- ビルドスクリプトヘルパ

```rust
pub fn wire(lake_build_dir: &str) -> Result<(), String> {
    // シム .so と handshake ファイルの絶対パスを解決。
    // `cargo:rustc-env=LEO4_SHIM_SO=…`
    //       `cargo:rustc-env=LEO4_HANDSHAKE_FILE=…`
    //       `cargo:rustc-env=LEO4_MANGLING_FILE=…`
    //       `cargo:rerun-if-changed=…` を発行。
    ...
}
```

これによりユーザの `main.rs` で `env!("LEO4_SHIM_SO")` が動く。マクロは
`LEO4_MANGLING_FILE` を (これも `env!` で) 読んで mangling テーブルを
解決する。

== 結合

完全な consumer クレート:

```
my-app/
  Cargo.toml         # [dependencies] leo4 = "..."; [build-dependencies] leo4-build = "..."
  build.rs           # leo4_build::wire(<path>)
  src/main.rs        # mod sample { leo4::import! { ... } } fn main() { ... }
```

`my-app/` から `cargo run` がラッパマクロ展開をビルドし、シム `.so` を
リンクし、ランタイム呼び出しがエンドツーエンドで動く。

= 第 VII 部 --- WIT 変換 (任意)

IDL は WIT の上位集合。どの leo4 IDL も Component Model ツールが
消費できる WIT ファイルへ下げられる。

== `leo4c lower`

`.leo4-schema` を読んで `.wit` ファイルを発行する小さな Rust CLI
(`crates/leo4c`)。変換:

- IDL `record R { f: u32 }` → WIT `record r { f: u32 }`。
- IDL `variant V { a, b(string) }` → WIT
  `variant v { a, b(string) }`。
- IDL `resource X` → WIT `resource x`。
- IDL `enum E { a, b }` → WIT `enum e { a, b }`。
- IDL `flags F { x, y }` → WIT `flags f { x, y }`。
- IDL `func f(_0: T) -> R;` → WIT
  `f: func(_0: t) -> r`。

WIT の自己再帰 variant は `resource` 型で表現する (WIT は variant
ペイロード内の直接自己再帰を許さない)。変換器が再帰を検出して置換する。

出力検証:

```
$ wasm-tools component wit <pkg>.wit  # parse + pretty-print
$ wit-bindgen markdown <pkg>.wit       # API ドキュメント生成
```

どちらもエラーなしで出力を受け入れるはずだ。

= 第 VIII 部 --- 相互再帰 + `Cyc<i>`

元プロジェクトの Phase 6。ここまで再帰は `Self` (1 つの宣言が自身を
再帰参照する) を通っていた。相互再帰には 2 つの宣言が互いを名で参照する
道が必要。

== IDL 文法追加

```
mutual_decl = "mutual" "{" nominal_decl nominal_decl { nominal_decl } "}" ";"
cyc_type    = "Cyc" "<" unsigned_int ">"
```

`mutual` ブロックは `Cyc<i>` 名前空間を共有する ≥ 2 個の nominal 宣言を
含む。どのメンバの中でも `Cyc<i>` はソース順序の `i` 番目のメンバを
指す。

== Mangling 規則

`Cyc<i>` → `c<i>c`、ここで `<i>` は ASCII 10 進インデックス。schema
hash は `Cyc<i>` トークン含む完全な正規化テキストで計算されるので、
メンバ順序の入れ替えはハッシュをローテートする。

== プラグイン作業

Lean プラグインは `InductiveVal.all` 配列で mutual クラスタを検出。
`iv.all.length > 1` なら `walkMutualGroup` 関数へディスパッチ:

1. 各メンバについて `mutualMembers = iv.all` 付きで `walkUserDecl` を
   呼ぶので、peer 参照が `Cyc<i>` に書き換わる。
2. 結果の `UserDecl` 配列を `UserDecl.mutual` で包む。

シム発行器の variant ヘルパハンドラは `Cyc<i>` ペイロードを拾い、
peer の `leo4_dec_<seg>` / `leo4_enc_<seg>` へクロスコールを発行。両
ヘルパは同じ翻訳単位にいて、シムヘッダ最上部の前方宣言で呼び出し点に
可視化される。

deriving ハンドラはクラスタごとに 1 つの `mutual partial def …
end` ブロックを、メンバごとに 1 つの `instance : LeanMarshal X` を発行。
クロス宣言ペイロード参照は peer の `<peer>._leo4_encode` /
`_decode` に直接ルーティング (型クラスディスパッチは未完成の
インスタンスを前方参照することになる)。

== Rust derive

Rust は同じモジュール内のトップレベル `impl` ブロック間の前方参照を
自由に許す。`leo4-abi/composites.rs` の `Box<T>` パススルー `LeanMarshal`
impl により、`Expr { Lit(u64), Seq(Box<Stmt>) }` のような再帰 Rust enum
が sized になる。`#[derive(LeanMarshal)]` は各 enum を独立に扱い、
サイクルはコンパイル時に解決する。

== 健全性チェック

mutual クラスタ付きのサンプルを着地:

```lean
mutual
  inductive Expr where
    | lit  (n : UInt64)
    | seq  (s : Stmt)
    deriving LeanMarshal
  inductive Stmt where
    | nop
    | block (e : Expr)
    deriving LeanMarshal
end

@[leo4_export]
def exprIsLit (e : Expr) : Bool := match e with | .lit _ => true | .seq _ => false
```

`lake exe leo4plugin Sample` 後、schema は次を含むはず:

```
mutual { variant Sample.Expr { lit(u64), seq(Cyc<1>) }; variant Sample.Stmt { nop, block(Cyc<0>) }; };
```

Rust 側は鏡 enum + 手書き (または derive) `LeanMarshal` impl を定義し、
マクロ経由で `exprIsLit` を呼ぶ。

= 第 IX 部 --- 非同期 io<T> + WASIp3

Phase 7。ユーザ向け API は両ターゲットで sync を維持する (2026-05-20 に
固定された設計判断による)。WASIp3 は sync wasm export 内部で非同期
wasip3 future を `block_on` できる。

== IDL 表面

Lean の `def f : IO α` はプラグインの `exprToIDLSubst` で
`IDLType.io α` に下る。正規 IDL はこれを `future<α>` (Phase 7 lift) と
レンダリング。Rust schema-idl パーサはパース時に `future<α>` を
`FuncDecl { effect: Async, ret: α }` に desugar するので、往復が対称に
保たれる。

== シム IO 展開

`IO α` export の Lean ラッパは C レベルで `lean_io_result α` を返す。
シムは呼び出しを包む:

```c
lean_object* io_res = leo4_lean__<mangled>(args);
if (!lean_io_result_is_ok(io_res)) {
    lean_dec(io_res); *ret_len = 0;
    return LEO4_ERR_IO_FAILED;
}
RetType r = scalarUnbox(lean_io_result_get_value(io_res));
lean_dec(io_res);
// r をエンコード...
```

`scalarUnbox` は cType ごとにディスパッチ: `lean_unbox_uint64` /
`lean_unbox_uint32` / `lean_unbox` / `lean_unbox_float` /
`lean_unbox_float32`。符号付き/符号なしは同じ C 幅を共有。呼び出し点での
キャストが符号解釈を保つ。

== WASIp3 姉妹

`sibling/leo4-wasip3/` のスタンドアロン Cargo プロジェクト。メイン
ワークスペースの *非*メンバ。stable Rust + `wasm32-wasip2` ターゲット
固定。`wasip3` クレート (WASIp3 API バインディングを wasip2 の Component
Model 上の互換シムとして出荷) に依存。

姉妹は `leo4_native::Lean::open` と類似の `leo4_wasip3::Lean::open`
を実装するが、ディスパッチは wasip3 host import (ホストが実装する WIT
ファイル定義) を通る。`futures::executor::block_on` がすべての async
import を駆動し、ユーザ向け Rust API は sync を保つ。

== 健全性チェック

`IO` 風 Sample export を着地:

```lean
@[leo4_export]
def asyncDouble (n : UInt64) : IO UInt64 := return n * 2
```

schema は `func asyncDouble(_0: u64) -> future<u64>` を示すはず。Rust
呼び出し側は `fn asyncDouble(n: u64) -> u64;` と書き、
`asyncDouble(21) == 42` を得る。

= 第 X 部 --- Mathlib 互換 carrier 型

Phase 8。leo4 は ROADMAP §8 により Mathlib 独立を維持 --- ランタイム
ライブラリは Mathlib を import しない。だが抽象 Mathlib 型 (`ℚ`,
`ZMod (2^128)`, `Complex ℝ`, `ℝ`) との往復が可能な carrier 型
(`LeanRat`, `LeanU128/I128`, `LeanComplexF*x2`, `LeanF16/BF16/F128`
nightly) は出荷する。

== ワイド整数

`Leo4.LeanU128 { lo : UInt64, hi : UInt64 }` と対応する `LeanI128`。
ワイヤは 16 バイト LE。`deriving LeanMarshal` のフィールド単位エンコード
が Rust の `u128::to_le_bytes()` と同じバイトストリームを生む。Rust の
マクロは生の `u128` を `rust_type_to_idl` 経由で `Leo4.LeanU128` IDL
形にマップ。

== 機械複素数

`Leo4.LeanComplexF{32,64}x2 { re, im : Float* }`。命名規則
`F<bits>x<components>` は後に quaternion (`xN=4`) / octonion (`xN=8`)
carrier まで拡張される。

== Nightly 浮動小数点

`LeanF16`, `LeanBF16`, `LeanF128` と対応する complex carrier、
`nightly-floats` cargo feature の後ろにゲート。Rust の `f16` / `f128`
プリミティブは nightly で
`#![cfg_attr(feature = "nightly-floats", feature(f16, f128))]`、`bf16`
にはまだ Rust ネイティブ・プリミティブがないのでビットパターンを `u16`
newtype として運ぶ。

Lean 側にはネイティブ `Float16` / `Float128` がない。carrier は生の
ビットパターン (`UInt16` または 2 つの `UInt64`) を包む。

== External marshal (`Rat`)

Lean コアの `Rat` はプラグインが下げられない proof-carrying フィールド
(`den_nz`, `reduced`) を持つ。`UserDecl.externalMarshal` 経路はこれらを
IDL レベルで不透明 blob として扱う。シム発行器が encode / decode を
Lean が発行する C 呼び出し可能ヘルパ (`leo4_marshal_Rat_dec` /
`leo4_marshal_Rat_enc`) に経由させる --- これらは
`Leo4.LeanMarshal.canonicalDecode/Encode` をラップする。シムは
`lean_alloc_sarray` + `leo4_memcpy` で `uint8_t* ⇄ ByteArray` のグルー
を担う。

== Mathlib ブリッジ

各 carrier には opt-in `Leo4.MathlibBridge.<Sub>` モジュールが付属。
ブリッジ:

- `Wide` --- `LeanU128/I128 ↔ Nat / Int / BitVec 128 / ZMod (2^128)`。
- `Complex` --- `LeanComplexF{32,64}x2 → ℂ` を `Float.toReal` で。逆方向
  `ℂ → LeanComplexF*x2` は `noncomputable` (Mathlib の ℝ には構成的な
  `→ Float` がない)。
- `NightlyFloats` --- IEEE-754 ビット・デコード `LeanF{16,BF16,128}
  → ℝ` を `Nat` フィールド抽出に対する直接算術で。逆方向は `Rat`
  (ℝ の計算可能部分集合) を経由し、IEEE 正しい
  round-to-nearest-even。
- `Rat` --- Lean コア `Rat` → `ℝ` / `ℂ` の全埋め込みを Mathlib の
  `Rat.cast` で。

丸めモード方針: IEEE-754 round-to-nearest-even (RTNE)。これは
`Float.div` とホスト FPU が実装するもの。だから抽象 Real の逆方向経路
が、ネイティブ・コードが既に行う往復と一貫している。

== 締めくくり

これでエンドツーエンドの leo4 実装ができた。次のステップは挑戦目標:
WIT 変換の精緻化、追加の Mathlib ブリッジ、安定化時の `wasm32-wasip3`
native ターゲット、消費者が必要とする時の schema-idl
`ConstraintExpr<Atom>` typed AST。

完全な参照実装は `github.com/Honey-Be/leo4` にある。進めながら比較
照合するとよい。そこのコミットメッセージは各ステップに名前を付け、
設計がそこに落ち着いた理由を説明している。

Happy hacking.
