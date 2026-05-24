#import "../../template/leo4-book.typ": book
#show: book.with(
  title: "leo4 — 처음부터 직접 구현하기",
  subtitle: "한국어판",
  author: "윤병익 (leo4 project)",
  lang: "ko",
)

= 머리말

이 책은 leo4를 백지에서 시작해 단계별로 빌드해 나가는 과정을 다룬다.
각 단계는 원본 프로젝트가 실제로 거쳐 온 순서와 동일하다. 책을 끝까지
따라오면 다음을 갖추게 된다.

- `@[leo4_export]` 속성과 `LeanMarshal` 타입클래스를 노출하는 Lean 4
  라이브러리.
- 사용자 패키지의 export를 발견하고, 안정적인 스키마 해시를 계산하고,
  심볼을 mangle하고, C 셰임 + 핸드셰이크 파일을 방출하는 Lake 플러그인.
- `libloading`으로 셰임을 적재하고, 호출을 canonical-ABI 바이트로 인코딩
  하고, mangling 테이블로 디스패치하고, 결과를 디코딩하는 Rust 워크스페이스
  --- 그리고 이 모든 것을 `fn add(a: u64, b: u64) -> u64;` 형태의 깔끔한
  선언 뒤로 숨기는 절차적 매크로.
- 선택적 확장: WIT 변환, 제네릭 레코드 지원, 상호 재귀, 비동기 `io<T>`,
  Mathlib 풍 carrier 타입.

이 책은 `SPEC/*.md`의 내용을 복제하지 않는다. 스펙은 규범이다. 이 책은
스펙을 향해 빌드해 나갈 뿐이다. 스펙과 본 책이 충돌한다면 스펙을 따른다.

== 준비물

- `lean-toolchain`에 고정된 버전과 일치하는 Lean 4 툴체인 (현재 v4.29.1).
- Rust 툴체인 ≥ 1.85 (Edition 2024).
- Lake 플러그인이 `leanc`를 통해 사용할 C 컴파일러 (`clang` 또는 `gcc`).
- `cargo`, `lake`, `just`, `jq`, `wasm-tools` (선택 WIT 장에서 필요).
- 집중할 수 있는 몇 시간. 파이프라인이 깊다. 점심 시간에 끝낼 수 없다.

== 구성

이 책은 phase ladder를 따른다 (원본 프로젝트의 `ROADMAP.md`). 각 부는
하나의 능력을 end-to-end로 완성한다. 매 부 끝에는 데모를 돌려서 무언가가
움직이는 것을 확인할 수 있다.

#table(
  columns: (auto, 1fr),
  table.header[*부*][*달성 내용*],
  [I],   [Lean 런타임 라이브러리와 `@[leo4_export]` 속성.],
  [II],  [Lake 플러그인 골격; 제네릭용 admit-set 알고리즘.],
  [III], [IDL: 타입, mangling, 스키마 해시. Lean과 Rust 사이의 안정 계약.],
  [IV],  [Canonical-ABI 마샬링: Lean 측 (`LeanMarshal` 타입클래스)
          + Rust 측 (`LeanMarshal` 트레이트).],
  [V],   [C 셰임 방출: export당 `leo4_call_<mangled>` 변환 단위.],
  [VI],  [Rust 적재기 (`leo4-mslean4`)와 `leo4::import!` proc-macro.],
  [VII], [WIT 변환 패스 및 `wasm-tools` 검증 (선택, 깔끔히 분리 가능).],
  [VIII],[상호 재귀 + `Cyc<i>`.],
  [IX],  [비동기 `io<T>`, WASIp3 자매 프로젝트.],
  [X],   [Mathlib 풍 carrier 타입과 브리지.],
)

어느 부 끝에서 멈춰도 동작하는 시스템이 남는다. 5부까지만 하면 스칼라와
문자열을 통한 Lean--Rust 왕복이 끝난다. 8부까지 하면 실무 코드가 요구할
법한 Phase 6 표면이 모두 갖춰진다.

= 1부 --- Lean 런타임 라이브러리

먼저 Lean 측 표면을 깎아낸다. 플러그인과 Rust 적재기는 모두 속성,
타입클래스, canonical-ABI 인코더/디코더 위에 올라간다. Lean 라이브러리는
이들의 거처다.

== 프로젝트 구성

Lake 패키지를 만든다.

```
lake/
  Leo4/
    Leo4.lean          -- 최상위 재내보내기
    Leo4/
      Syntax.lean      -- leo4_constraint 구문 카테고리
      Export.lean      -- @[leo4_export] 속성
      Marshal.lean     -- LeanMarshal 타입클래스 + LeanError
      Resource.lean    -- LeanResource 마커
      Builtins.lean    -- 기본형용 LeanMarshal 인스턴스
      Deriving.lean    -- deriving LeanMarshal 핸들러
      Build.lean       -- 사용자용 빌드 헬퍼
    lakefile.lean
```

`Leo4` 라이브러리는 모든 하위 패키지가 `require`한다. 최상위 import는
최소화해 둔다.

== `LeanError` 정의

Lean 측의 모든 실패 가능한 작업은 에러 코드 + 메시지를 반환할 방법이
필요하다. 평탄한 구조로 둔다.

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

에러 코드는 `SPEC/canonical-abi.md` §13을 따른다. `0x00000001`은
`decodeError`, `0x00000005`는 핸드셰이크 불일치, `0x00000007`은 반환 버퍼
부족, `0x00000064`는 unimplemented. 같은 파일에 `def` 상수로 두자.

== `@[leo4_export]` 속성

`@[leo4_export]`는 빈 마커 속성이다. 플러그인은 속성 확장을 조회해
태깅된 선언을 찾는다. Lean 4에는 `registerBuiltinAttribute`가 있다.

```lean
import Lean
namespace Leo4

initialize leo4ExportAttr : Lean.TagAttribute ←
  Lean.registerTagAttribute `leo4_export
    "marks a declaration as a leo4 boundary export"

end Leo4
```

검증: 다른 모듈에 자명한 export 하나를 두고
`leo4ExportAttr.hasTag (← getEnv) ``YourModule.add`가 `true`를
돌려주는지 확인한다.

== `LeanMarshal` 타입클래스

`LeanMarshal`은 경계를 넘는 모든 값 타입이 구현하는 타입클래스다. 인코드
측은 `ByteArray`에 바이트를 덧붙이고, 디코드 측은 그것에서 읽어
`(값, 다음 오프셋)`을 + 에러 경로와 함께 반환한다.

```lean
namespace Leo4

class LeanMarshal (T : Type) where
  canonicalEncode : T → ByteArray → ByteArray
  canonicalDecode : ByteArray → Nat → Except LeanError (T × Nat)

end Leo4
```

Lean의 `ByteArray`는 패킹된 `Array UInt8`이다. `Nat` 오프셋은 안전을
위해 무경계 인덱싱을 준다. 와이어 포맷의 길이 prefix가 실질적인 경계를
들고 있다.

== 기본형 인스턴스

스칼라부터: `UInt8`, `UInt16`, `UInt32`, `UInt64`, `Int8`...`Int64`,
`Float`, `Float32`, `Bool`, `Char`. 각각 little-endian 바이트로 인코드하고
같은 방식으로 디코드한다.

대표 구현, `UInt32`:

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

모든 기본형에 반복. 그 다음 복합형 인스턴스:

- `String` --- `u32 len + utf-8 bytes`.
- `List T` --- `u32 len + N elements`.
- `Option T` --- `u8 disc + payload`.
- `Except E T` --- `u8 disc + payload`.
- `α × β` --- 두 원소 이어붙이기.
- `Nat` (`bignat`) --- `u32 limb count + LE u64 limbs`.
- `Int` (`bigint`) --- `u8 sign + bignat magnitude`.

각 인스턴스는 양방향. 디코드 측 경계 체크는 공격적으로. 와이어 입력은
정의상 신뢰할 수 없다.

== Deriving 핸들러

Rust의 `#[derive(LeanMarshal)]`과 Lean의 `deriving LeanMarshal`은 모두
사용자 정의 타입에 대해 필드별 encode/decode를 합성한다. Lean 측은
`registerDerivingHandler`로 한다.

```lean
namespace Leo4.Deriving

open Lean Elab Command Meta

private def mkLeanMarshalHandler (declNames : Array Name)
    : CommandElabM Bool := do
  let env ← getEnv
  for declName in declNames do
    let some (.inductInfo indVal) := env.find? declName
      | return false
    -- 형태별 분기: 단일 ctor → 레코드, 모두 무인자 다중 ctor → enum,
    -- 혼합 → variant.
    -- (자세한 내용은 발표된 프로젝트의 Deriving.lean.)
    pure ()
  return true

initialize
  registerDerivingHandler ``Leo4.LeanMarshal mkLeanMarshalHandler

end Leo4.Deriving
```

핸들러 본문이 어려운 부분이다. 귀납형의 ctor들을 순회해 encode arm
(ctor당 한 match-arm, 판별자 push 후 각 필드 encode)을 만들고, decode
arm (판별자별 한 arm, 안쪽 필드 추출)을 만든다. Phase 6의 mutual 지원을
위해, `mutual ... end` 클러스터의 모든 멤버는 한 `mutual ... end` 블록의
`partial def` encoder/decoder + 멤버당 한 인스턴스로 묶인다.

먼저 단일 형태(예: 레코드만)부터 구현하자. enum / variant / mutual은
이후 단계에서 추가하면 된다.

== 검증

Leo4 라이브러리 밖에 Lean 파일을 작성한다.

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

16바이트(두 `f64`)가 인코드되고 다시 왕복되는 것을 확인. 그렇지 않다면
다음으로 넘어가기 전에 인코더/디코더를 고친다.

= 2부 --- Lake 플러그인 골격

Lake 플러그인은 `lake build` 후에 도는 `lean_exe`다. 사용자 패키지의
모든 `@[leo4_export]` 정의를 순회하며, 제네릭의 admit-set을 계산하고,
학습 자료 3장에 나열된 산출물을 방출한다.

== 프로젝트 구성

```
lake/
  Leo4Plugin/
    Leo4Plugin.lean      -- 최상위
    Leo4Plugin/
      AdmitSet.lean      -- IDLType + UserDecl ADT, admit-set 알고리즘
      Mangling.lean      -- mangleType, 스키마 해시
      Emit.lean          -- 파일 라이터, JSON 형태
      Main.lean          -- runPlugin 드라이버
    Main.lean            -- exe 엔트리
    lakefile.lean
```

`lakefile.lean`은 패키지를 선언하고, `Leo4` (1부의 런타임 라이브러리)를
`require`하며, 루트 모듈이 `Main.lean`인 `lean_exe leo4plugin`을 노출한다.

== Export 발견

플러그인은 독립 실행 파일로 돈다. 사용자의 모듈명을 명령행 인자로 받는다.

```
$ lake exe leo4plugin Sample
```

엔트리는 `Lean.importModules (loadExts := true)`로 사용자 컴파일 모듈을
적재한 다음, 환경에서 `@[leo4_export]` 태깅 decl을 찾는다.

```lean
def gatherExports (env : Environment) : Array Name := Id.run do
  let mut out : Array Name := #[]
  for (n, _) in env.constants do
    if Leo4.leo4ExportAttr.hasTag env n then
      out := out.push n
  return out
```

이렇게 정렬된 `Name` 목록이 나온다. 각각에 대해 분석기는 함수의 타입을
가져와서 제네릭 매개변수 / 값 매개변수 / 반환 타입으로 쪼개고, 각각을
IDL로 낮춘다.

== IDLType ADT

플러그인의 IDL 표현은 Rust 측 `schema-idl::IDLType`을 거울처럼 닮은
Lean 귀납형이다. `AdmitSet.lean`에 둔다.

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

생성자 하나하나가 정규 IDL이다. Rust 측은 (Rust 명명 관행을 제외하면)
이것을 그대로 거울로 갖는다. mangling 규칙(5장)은 각각을 안정 ASCII
문자열로 매핑한다.

`UserDecl` ADT는 nominal 타입 선언을 모은다.

```lean
inductive UserDecl where
  | record   (fqn) (generics : Array Name) (fields : Array (Name × IDLType))
  | enumT    (fqn) (cases : Array Name)
  | variant  (fqn) (generics : Array Name) (cases : Array (Name × Array IDLType))
  | resource (fqn) (generics : Array Name)
  | mutual   (members : Array UserDecl)
  | externalMarshal (fqn) (generics : Array Name)
```

두 추가 생성자(`mutual`, `externalMarshal`)는 6단계 / 10단계에서 합류한다.
ADT는 처음부터 완전하게 두자.

== Export 순회

태깅된 `Name`마다 `ConstantInfo`를 가져와, 타입을 telescope 분해하고,
각 바인더를 낮춘다.

```lean
def analyzeExport (n : Name) : MetaM (Option ExportAnalysis) := do
  let env ← getEnv
  let some info := env.find? n | return none
  Meta.forallTelescope info.type fun args body => do
    -- 바인더 분류:
    --   - 암시적 kind-typed → 제네릭 타입 매개변수
    --   - 암시적 value-typed → 소거된 값 매개변수
    --   - inst-implicit → 타입클래스 제약
    --   - 명시적 → 경계 값 매개변수
    -- 그 후 각 값 매개변수 타입을 exprToIDLSubst로 낮춤.
    sorry
```

`exprToIDLSubst`는 재귀 타입 변환기다. Lean `Expr`과 치환 맵(제네릭
바인더 → 구체 `IDLType`)을 받아 해당 `IDLType` 또는 변환 불가 시 `none`
을 돌려준다. `List`, `Option`, `Except`, `Prod`, `IO`, Self 단락 특수화.
사용자 정의 귀납형은 형태에 따라 `record`/`variant`/`enumT`/`resource`로
낮춰진다.

핵심 디테일: `Meta.forallTelescope`를 (reduce 없이) 사용해 원래의
`IO α` 형태가 살아남도록 한다. reducing 변종은 `IO α = IO.RealWorld →
EStateM …`로 펼쳐서 가짜 `IO.RealWorld` 매개변수를 노출한다.

== Admit-set 알고리즘

제네릭 매개변수가 있는 export는, 플러그인이 바인더 제약을 만족하는 모든
구체화를 열거한다. 각 조합에 대해 별도의 IDL 시그니처와 mangled 이름을
만든다. 알고리즘:

1. 각 제네릭 `T_i`에 대해 admit-set 결정: 가질 수 있는 `IDLType` 값들의
   집합. 기본: 모든 기본형 (`unboundedAdmitSet`). 클래스 제약 있음:
   각 클래스의 `classAdmitSet`과 교집합.
2. 데카르트 곱 계산. 각 튜플이 한 구체화.
3. 각 구체화마다 매개변수 타입에 치환하고 `paramInfo` 배열 생성.

팬텀 제네릭 (어디서도 참조되지 않는 바인더)은 조합 폭발을 건너뛴다 ---
팬텀 슬롯을 `none`으로 둔 단일 구체화만 방출.

이 알고리즘은 발표된 코드베이스의 `Main.lean` `analyzeExport`에 있다.
재구현 전에 한 번 읽자. 엣지 케이스(고차 제네릭, 값 제네릭, Self
재귀형 안의 제네릭 인자)가 시간을 잡아먹는다.

= 3부 --- Mangling과 스키마 해시

`IDLType` 값들 + export 목록 + 발견된 사용자 타입들이 있으면, 스키마
해시가 입력으로 쓸 안정 텍스트 형식을 생성할 수 있다.

== `mangleType`

`mangleType : IDLType → String`은 Lean과 Rust 사이에서 바이트 단위로
동일하다. 각 생성자가 고정 토큰으로 매핑된다.

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

전체 규칙은 `SPEC/mangling.md` §2.

== 전체 mangled 이름

```
leo4__<pkg_seg>__<iface>__<fname>__<arg_mangles>__h<schema_hash>
```

`arg_mangles`는 각 매개변수 타입의 mangle 형태를 밑줄로 잇는다. 스키마
해시는 *정규화된 IDL 형식*(텍스트)의 FNV-1a-64를 13자 base32lc
(소문자, 패딩 없음)로 렌더링한 것.

FNV-1a-64는 단순하다. offset basis `0xCBF29CE484222325`, prime
`0x00000100000001B3`, 각 바이트 XOR 후 곱. Base32lc는 알파벳
`abcdefghijklmnopqrstuvwxyz234567` (RFC 4648 소문자, 패딩 없음).

== 정규 IDL 렌더링

`renderCanonical : Config → Array UserDecl → Array Member → Bool → String`은
다음 텍스트를 만든다.

```
package leo4-sample;
interface Sample {
  record Sample.Point { x: f64, y: f64 };
  variant Sample.Tree { leaf, node(Self, Self) };
  func add(_0: u64, _1: u64) -> u64;
  func midpoint(_0: Sample.Point, _1: Sample.Point) -> Sample.Point;
}
```

두 모드: `pretty := true` (개행, 들여쓰기) --- 디스크의 `.leo4-schema`
파일용; `pretty := false` (축약, 토큰 간 단일 공백) --- 스키마 해시
입력용.

사용자 decl은 같은 밴드 내에서 FQN으로 정렬 (밴드 0: record/enum, 밴드 1:
resource, mutual 클러스터는 밴드 0이며 원본 순서 보존). 함수는 이름으로
정렬. 결정성은 협상 불가 --- 해시는 바이트 단위로 동일한 출력에 의존한다.

해시 입력은 *축약* 형태. UTF-8 바이트에 FNV-1a를 돌려 `UInt64`를 얻고,
big-endian으로 base32 문자열로 변환해 접미사로 쓴다.

== 검증

작은 픽스처 (Lean 측):

```lean
@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b
```

`lake exe leo4plugin Sample`을 돌리고 다음을 확인:

- `tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-schema`가
  합리적인 텍스트 파일.
- `tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-handshake`의
  `schema_hash` JSON 필드에 16자 base32lc 스키마 해시.
- 재실행 시 해시가 안정.

그 후 Rust 거울 (`crates/schema-idl/`)을 병행해 구현하고, Lean 출력을
`leo4c mangle <schema>` 출력과 비교하는 cross-impl 하니스
(`tests/mangling/`)를 추가. 둘은 바이트 단위로 일치해야 한다.

= 4부 --- Canonical-ABI 마샬링

Lean 라이브러리에는 `LeanMarshal` 타입클래스가 있다. Rust 측에도 짝이
되는 트레이트가 필요하다. 양 측이 공유하는 모든 값 타입에 대해 동일한
바이트를 만들어야 한다.

== Rust 트레이트

```rust
pub trait LeanMarshal: Sized + 'static {
    fn canonical_encode(&self, buf: &mut Vec<u8>);
    fn canonical_decode(buf: &[u8], off: usize)
        -> Result<(Self, usize), LeanError>;
}
```

인코드는 `Vec<u8>` (필요 시 성장), 디코드는 `&[u8] + off` (Lean 측과 동일).
`LeanError`는 `u32` 코드 + `String` 디테일로, Lean의 `Leo4.LeanError`와
일치한다.

== 기본형 impl

각 Rust 기본형에 대해 직접 impl 작성:

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

모든 기본형에 반복. 정확한 LE 동작이 중요하다 --- Lean의
`(n.toUInt8, ..., (n >>> 24).toUInt8)` 체인이 만드는 바이트와 동일해야.

== 적합성 하니스

양측은 바이트 단위로 일치해야 한다. 픽스처를 만든다.

```
tests/conformance/
  fixtures/
    u32.lean       -- Leo4.LeanMarshal로 `u32 42`를 바이트로 방출
    u32.rs         -- leo4-abi로 `42u32`를 바이트로 방출
    point.lean     -- 레코드 예
    point.rs       -- 동일
    ...
  run.sh
```

`run.sh`는 동일한 논리값으로 양 픽스처를 돌려 바이트 출력을 비교하고,
짝이 어긋나면 실패. 출시 전에 미묘한 바이트 순서 실수를 잡는 테스트다.

타입당 최소 한 픽스처: 모든 기본형, 모든 복합 형태(list, option, result,
tuple), 그리고 최소 두 사용자 타입(record, variant).

= 5부 --- C 셰임 방출

C 셰임은 Lean의 native ABI(`lean_object*`, `lean_alloc_ctor`,
`lean_io_result_*`, …)와 canonical ABI의 바이트 스트림이 만나는 자리.
플러그인은 패키지당 하나의 `.c` 파일을 생성하며, export × 구체화당
하나의 `LEO4_EXPORT int32_t leo4_call_<mangled>(...)` 엔트리를 둔다.

== 셰임 소스 구조

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

/* 헬퍼별 extern 선언 */
extern uint64_t leo4_lean__leo4__sample__Sample__add__u64_u64__h<hash>(uint64_t, uint64_t);

LEO4_EXPORT int32_t leo4_call_leo4__sample__Sample__add__u64_u64__h<hash>(
    leo4_arena_t* arena,
    const uint8_t* args_ptr, size_t args_len,
    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)
{
    /* 인자 디코드 */
    /* 호출 */
    /* 반환 인코드 */
}
```

시그니처는 고정 (`SPEC/canonical-abi.md` §14). 적재기는 dlsym으로 이를
바인딩하고, 매크로는 여기로 호출하는 Rust 코드를 생성한다.

== 타입별 핸들러

셰임 방출기의 핵심 자료구조는 `TyHandler`.

```lean
private structure TyHandler where
  cType        : String   -- 예: "uint64_t"
  externCType  : String   -- extern 선언의 C 타입
  ownsRef      : Bool     -- 마지막에 lean_dec 필요?
  scalarKind   : Option String  -- ctor 접근자용 "uint8" 등
  ctorScalarSz : Nat
  decodeBlock  : String → String → String  -- (var, cleanup) → C
  encodeBlock  : String → String → String
  boxExpr      : String → String  -- value → lean_object*
  unboxExpr    : String → String  -- lean_object* → value
```

각 IDL 타입마다 방출기가 `TyHandler`를 해결한다. 스칼라는 일반
`scalarHandler` 사용. 문자열은 `stringHandler` (런타임 헬퍼로 위임).
List / option / result / tuple은 고차 --- `listHandler ih`는 내부 타입의
핸들러를 받아 감싼다.

사용자 정의 레코드는 필드 핸들러로부터 `recordHandler` 생성. variant는
자체 방출기가 (fqn, args) 구체화당 두 헬퍼 함수(예:
`leo4_dec_Sample_Tree` / `leo4_enc_Sample_Tree`)를 만들고, 각각 disc +
페이로드를 처리.

variant 안의 Self 참조는 동일 헬퍼를 재귀 호출. mutual 클러스터는
`Cyc<i>` 참조를 사용해 방출 시점에 peer 헬퍼로 해소(8장).

== 메인 렌더 루프

```lean
def renderOneShim (cfg userDecls a schemaHash params ret) : String :=
  let mangled := mangle cfg.pkg cfg.iface a.fname (params.map ...) schemaHash
  let entry  := s!"leo4_call_{mangled}"
  let helper := s!"leo4_lean__{mangled}"
  -- paramHs : Array TyHandler 구성 (handlerFor로 params에서).
  -- retH    : TyHandler 구성 (ret에서).
  -- 핸들러 중 `none`이 있으면 LEO4_ERR_UNIMPLEMENTED 스텁 방출.
  -- 아니면 디코드 → 호출 → 인코드 본문 전체 방출.
  ...
```

export당 대략 30--100줄의 생성 C. 그 결과를 `leanc` (Lean의 include /
라이브러리 경로가 미리 설정된 clang)로 컴파일해 `.so`를 만든다.

== 검증

`lake exe leo4plugin Sample` 후 `<pkg>.leo4-shim.c`를 확인한다.
스칼라 `add(u64, u64) -> u64`는 다음처럼 보여야 한다:

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

이 `.c` + Lean 래퍼 모듈의 `.c`에 `leanc`(또는 적절한 플래그의 `cc`)를
구동해 `<pkg>.leo4-shim.so`를 만든다. 사용자 패키지의 `.so`(lake build의
`precompileModules`에서 나온 것)를 RPATH로 링크해 래퍼가 사용자
컴파일 export를 호출할 수 있게 한다.

= 6부 --- Rust 적재기와 `import!` 매크로

셰임 `.so`, 핸드셰이크 파일, mangling 테이블이 있다. 이제 Rust 측이
이들을 바인딩한다.

== `leo4-mslean4` --- 적재기

`crates/leo4-mslean4/`는 `Lean::open`을 노출한다.

```rust
pub struct Lean { /* libloading::Library + 메타 */ }

impl Lean {
    pub fn open(
        so_path: impl AsRef<Path>,
        handshake_path: impl AsRef<Path>,
    ) -> Result<Self, LeanError> {
        // 1. 핸드셰이크 JSON 읽음; schema_hash + wrapper_init_symbol 추출.
        // 2. Rust 측 상수와 schema_hash 검증.
        // 3. libloading으로 `Library::new(so_path)`.
        // 4. 프로세스당 한 번 Lean 런타임 초기화
        //    (`lean_initialize_runtime_module`, 그 후 래퍼의
        //    `initialize_<X>` 심볼).
        // 5. mangled 심볼별 함수 포인터 캐시.
        ...
    }

    pub fn call_shim(
        &self,
        mangled_body: &str,
        args: &[u8],
        ret: &mut [u8],
    ) -> Result<usize, LeanError> {
        // dlsym으로 `leo4_call_<mangled>` 조회 (캐시).
        // (arena=NULL, args_ptr, args_len, ret_ptr, ret_cap, &ret_len)으로
        // 호출. int32_t 상태를 Result로 변환.
        ...
    }
}
```

런타임 init은 `std::sync::Once`로 프로세스당 한 번. 래퍼 모듈의
`initialize_<X>` 심볼은 `lean_io_result_is_ok` 방식 성공을 돌려준다.
사용자 호출 디스패치 전에 체크.

== `leo4-macros-backend` --- 매크로 확장기

`leo4::import!`는 함수형 절차 매크로 (`#[proc_macro]`). extern-block
같은 입력을 파싱하고, 빌드 시 mangling JSON에서 각 `fn`을 찾아 Rust
래퍼를 방출한다.

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

매크로의 일은 올바른 `MANGLED_BODY`를 고르는 것. `leo4-build`가 설정한
`LEO4_MANGLING_FILE`을 읽고 함수명 + 각 인자의 IDL 형식
(`rust_type_to_idl`로 계산)을 매칭. 다중 구체화 제네릭 export의 경우
`#[leo4(args = "u64,str")]` 속성으로 사용자가 명시 선택 가능.

== `leo4-build` --- 빌드 스크립트 헬퍼

```rust
pub fn wire(lake_build_dir: &str) -> Result<(), String> {
    // 셰임 .so와 핸드셰이크 파일의 절대 경로 해석.
    // `cargo:rustc-env=LEO4_SHIM_SO=…`
    //       `cargo:rustc-env=LEO4_HANDSHAKE_FILE=…`
    //       `cargo:rustc-env=LEO4_MANGLING_FILE=…`
    //       `cargo:rerun-if-changed=…` 방출.
    ...
}
```

이 덕에 사용자 `main.rs`에서 `env!("LEO4_SHIM_SO")`이 동작한다. 매크로는
`LEO4_MANGLING_FILE`을 (역시 `env!`로) 읽어 mangling 테이블 해소.

== 통합

완전한 소비자 크레이트:

```
my-app/
  Cargo.toml         # [dependencies] leo4 = "..."; [build-dependencies] leo4-build = "..."
  build.rs           # leo4_build::wire(<path>)
  src/main.rs        # mod sample { leo4::import! { ... } } fn main() { ... }
```

`my-app/`에서 `cargo run`이 래퍼 매크로 확장을 빌드하고, 셰임 `.so`를
링크하고, 런타임 호출이 end-to-end로 동작한다.

= 7부 --- WIT 변환 (선택)

IDL은 WIT의 상위 집합이다. 어떤 leo4 IDL도 Component Model 도구가 소비할
수 있는 WIT 파일로 낮출 수 있다.

== `leo4c lower`

`.leo4-schema`를 읽어 `.wit` 파일을 방출하는 작은 Rust CLI
(`crates/leo4c`). 변환:

- IDL `record R { f: u32 }` → WIT `record r { f: u32 }`.
- IDL `variant V { a, b(string) }` → WIT
  `variant v { a, b(string) }`.
- IDL `resource X` → WIT `resource x`.
- IDL `enum E { a, b }` → WIT `enum e { a, b }`.
- IDL `flags F { x, y }` → WIT `flags f { x, y }`.
- IDL `func f(_0: T) -> R;` → WIT
  `f: func(_0: t) -> r`.

WIT의 자기재귀 variant는 `resource` 타입으로 표현(WIT는 variant 페이로드의
직접 자기재귀를 허용하지 않음). 변환기가 재귀를 감지해 치환.

출력 검증:

```
$ wasm-tools component wit <pkg>.wit  # 파싱 + 정렬 출력
$ wit-bindgen markdown <pkg>.wit       # API 문서 생성
```

둘 다 오류 없이 출력을 수용해야 한다.

= 8부 --- 상호 재귀 + `Cyc<i>`

원본 프로젝트의 6단계. 여기까지는 재귀가 `Self`(한 선언이 자신을 다시
참조)로 갔다. 상호 재귀는 두 선언이 서로를 이름으로 참조할 길이 필요하다.

== IDL 문법 추가

```
mutual_decl = "mutual" "{" nominal_decl nominal_decl { nominal_decl } "}" ";"
cyc_type    = "Cyc" "<" unsigned_int ">"
```

`mutual` 블록은 `Cyc<i>` 네임스페이스를 공유하는 ≥ 2개의 nominal 선언을
담는다. 어떤 멤버 안에서든 `Cyc<i>`는 소스 순서 `i`번째 멤버를 가리킨다.

== Mangling 규칙

`Cyc<i>` → `c<i>c`, 여기서 `<i>`는 ASCII-십진 인덱스. 스키마 해시는
`Cyc<i>` 토큰 포함 전체 정규화 텍스트로 계산되어, 멤버 순서를 바꾸면
해시가 회전한다.

== 플러그인 작업

Lean 플러그인은 `InductiveVal.all` 배열로 mutual 클러스터를 감지.
`iv.all.length > 1`이면 `walkMutualGroup` 함수로 디스패치:

1. 각 멤버에 대해 `mutualMembers = iv.all`로 `walkUserDecl` 호출 →
   peer 참조가 `Cyc<i>`로 재기록.
2. 결과 `UserDecl` 배열을 `UserDecl.mutual`로 감쌈.

셰임 방출기의 variant 헬퍼 핸들러는 `Cyc<i>` 페이로드를 가져와 peer의
`leo4_dec_<seg>` / `leo4_enc_<seg>`에 cross-call. 두 헬퍼는 동일 변환
단위에 있고, 셰임 헤더 최상단의 전방선언으로 호출 시점에 가시화.

deriving 핸들러는 클러스터당 한 `mutual partial def … end` 블록과,
멤버당 한 `instance : LeanMarshal X`를 방출. 교차 decl 페이로드 참조는
peer의 `<peer>._leo4_encode` / `_decode`로 직접 라우팅 (타입클래스
디스패치는 미완성 인스턴스를 전방 참조하게 됨).

== Rust derive

Rust는 동일 모듈의 최상위 `impl` 블록 사이 전방 참조를 자유롭게 허용.
`leo4-abi/composites.rs`의 `Box<T>` pass-through `LeanMarshal` impl은
`Expr { Lit(u64), Seq(Box<Stmt>) }` 같은 재귀 Rust enum이 sized가 되도록
한다. `#[derive(LeanMarshal)]`은 각 enum을 독립적으로 처리하고, 사이클은
컴파일 시 해소된다.

== 검증

mutual 클러스터 샘플:

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

`lake exe leo4plugin Sample` 후 스키마는 다음을 포함해야 한다:

```
mutual { variant Sample.Expr { lit(u64), seq(Cyc<1>) }; variant Sample.Stmt { nop, block(Cyc<0>) }; };
```

Rust 측은 거울 enum + 손수 (또는 derive) `LeanMarshal` impl을 정의하고
매크로를 통해 `exprIsLit`를 호출한다.

= 9부 --- 비동기 io<T> + WASIp3

7단계. 사용자 대면 API는 두 타깃 모두 sync로 유지 (2026-05-20에 못박힌
설계 결정에 따라). WASIp3는 sync wasm export 내부에서 비동기 wasip3
future를 `block_on`할 수 있게 해 준다.

== IDL 표면

Lean의 `def f : IO α`는 플러그인의 `exprToIDLSubst`에서 `IDLType.io α`로
낮춰진다. 정규 IDL은 이를 `future<α>`(Phase 7 lift)로 렌더링한다. Rust
schema-idl 파서는 파싱 시 `future<α>`를 `FuncDecl { effect: Async, ret:
α }`로 desugar해 왕복이 대칭으로 유지된다.

== 셰임 IO 풀기

`IO α` export의 Lean 래퍼는 C 수준에서 `lean_io_result α`를 반환. 셰임은
호출을 다음과 같이 감싼다.

```c
lean_object* io_res = leo4_lean__<mangled>(args);
if (!lean_io_result_is_ok(io_res)) {
    lean_dec(io_res); *ret_len = 0;
    return LEO4_ERR_IO_FAILED;
}
RetType r = scalarUnbox(lean_io_result_get_value(io_res));
lean_dec(io_res);
// r 인코드...
```

`scalarUnbox`는 cType별 디스패치: `lean_unbox_uint64` /
`lean_unbox_uint32` / `lean_unbox` / `lean_unbox_float` /
`lean_unbox_float32`. 부호/무부호는 동일한 C 너비를 공유; 호출 시점의
캐스트가 부호 해석을 보존한다.

== WASIp3 자매

`sibling/leo4-wasip3/`의 독립 Cargo 프로젝트, 메인 워크스페이스의
*비*멤버. stable Rust + `wasm32-wasip2` 타깃 고정; `wasip3` 크레이트
(WASIp3 API 바인딩을 wasip2의 Component Model 위 호환 셰임으로 제공)에
의존.

자매는 `leo4_mslean4::Lean::open`과 유사한 `leo4_wasip3::Lean::open`을
구현하지만, 디스패치는 wasip3 host import(호스트가 구현하는 WIT 파일에
정의)를 통한다. `futures::executor::block_on`이 모든 async import를
구동하면서 사용자 대면 Rust API는 sync로 유지된다.

== 검증

`IO` 풍 Sample export 적용:

```lean
@[leo4_export]
def asyncDouble (n : UInt64) : IO UInt64 := return n * 2
```

스키마에 `func asyncDouble(_0: u64) -> future<u64>`가 보여야 한다. Rust
호출자는 `fn asyncDouble(n: u64) -> u64;`를 적고 `asyncDouble(21) == 42`
를 얻는다.

= 10부 --- Mathlib 호환 carrier 타입

8단계. leo4는 ROADMAP §8에 따라 Mathlib 독립 유지 --- 런타임 라이브러리는
Mathlib을 import하지 않는다. 그러나 추상 Mathlib 타입(`ℚ`,
`ZMod (2^128)`, `Complex ℝ`, `ℝ`)으로/에서 왕복 가능한 carrier 타입
(`LeanRat`, `LeanU128/I128`, `LeanComplexF*x2`, `LeanF16/BF16/F128`
nightly)을 출하한다.

== 와이드 정수

`Leo4.LeanU128 { lo : UInt64, hi : UInt64 }`와 짝 `LeanI128`. 와이어는
16바이트 LE; `deriving LeanMarshal`의 필드별 인코드가 Rust의
`u128::to_le_bytes()`와 동일 바이트 스트림을 만든다. Rust 매크로는
순수 `u128`을 `rust_type_to_idl`을 통해 `Leo4.LeanU128` IDL 형식으로
매핑.

== 기계 복소수

`Leo4.LeanComplexF{32,64}x2 { re, im : Float* }`. 명명 관행
`F<bits>x<components>`는 이후 quaternion(`xN=4`) / octonion(`xN=8`)
carrier까지 확장된다.

== Nightly 부동소수점

`LeanF16`, `LeanBF16`, `LeanF128` 및 짝 complex carrier들,
`nightly-floats` cargo feature 뒤로 격리. Rust의 `f16` / `f128` 기본형은
nightly via `#![cfg_attr(feature = "nightly-floats", feature(f16, f128))]`;
`bf16`은 아직 Rust native 기본형이 없으므로 비트 패턴을 `u16` newtype으로
운반.

Lean 측에는 native `Float16` / `Float128`이 없다; carrier들은 원시 비트
패턴(`UInt16` 또는 두 `UInt64`)을 감싼다.

== External marshal (`Rat`)

Lean core `Rat`은 플러그인이 낮출 수 없는 증명-운반 필드(`den_nz`,
`reduced`)를 갖는다. `UserDecl.externalMarshal` 경로는 IDL 수준에서
이들을 불투명 blob으로 처리; 셰임 방출기가 encode/decode를 Lean이 방출한
C-호출 가능 헬퍼(`leo4_marshal_Rat_dec` / `leo4_marshal_Rat_enc`)를 통해
라우팅 --- 이들이 `Leo4.LeanMarshal.canonicalDecode/Encode`를 감싼다.
셰임은 `lean_alloc_sarray` + `leo4_memcpy`로 `uint8_t* ⇄ ByteArray` 접착.

== Mathlib 브리지

각 carrier에는 opt-in `Leo4.MathlibBridge.<Sub>` 모듈이 딸려 있다. 브리지:

- `Wide` --- `LeanU128/I128 ↔ Nat / Int / BitVec 128 / ZMod (2^128)`.
- `Complex` --- `LeanComplexF{32,64}x2 → ℂ` via `Float.toReal`. 역방향
  `ℂ → LeanComplexF*x2`는 `noncomputable` (Mathlib의 ℝ는 구성적
  `→ Float`이 없음).
- `NightlyFloats` --- IEEE-754 비트 디코드 `LeanF{16,BF16,128} → ℝ`,
  `Nat` 필드 추출에 대한 직접 산술로. 역방향은 `Rat`(ℝ의 계산 가능
  부분집합)을 거쳐 IEEE 올바른 round-to-nearest-even.
- `Rat` --- Lean core `Rat` → `ℝ` / `ℂ` 전체 임베딩, Mathlib `Rat.cast`로.

라운딩 모드 정책: IEEE-754 round-to-nearest-even (RTNE). `Float.div`와
호스트 FPU가 구현하는 것이므로, 추상 Real 역방향 경로가 native 코드의
왕복과 일관되게 유지된다.

== 마무리

이제 end-to-end leo4 구현이 있다. 다음 발자국은 도전 목표: WIT 변환
정제, 추가 Mathlib 브리지, 안정 시점의 `wasm32-wasip3` 네이티브 타깃,
소비자가 필요할 때의 schema-idl `ConstraintExpr<Atom>` typed AST.

완전한 참조 구현은 `github.com/Honey-Be/leo4`에 있다. 진행하면서 비교
확인. 그곳의 커밋 메시지는 각 단계를 명명하고 설계가 그렇게 자리잡은
이유를 설명한다.

즐거운 해킹.

= 업데이트 — 2026-05-24

reference checkout 이 2026-05-24 이후라면 구현 순서가 몇
부분 바뀜. core architecture 변경 0 — 모두 원래 phase
ladder 가 TODO 로 열거하던 v1.0-RC gap 들을 닫는 작업.

== OX6: PEG 기반 Lean 4 parser 를 sibling crate 로

`leo4-oxilean-build` 내 OX3 / OX4 textual pre-rewrite
chain (`lean4_normalize` 등) 은 임시방편이었고
operator precedence / string interpolation / ctor name
resolution 모두 진짜 grammar 작업이 필요했음.
`sibling/leo4-lean4-parse/` 에 `peg` crate 로 sibling
구축. `def NAME … := VALUE` 부터 시작, PEG 의
`precedence!` 로 expression precedence, 이후 Lean 4
surface form 들을 하나씩 (총 약 25 sub-step) 쌓아 올림.
AST shape 은 `oxilean-parse` v0.1.2 를 mirror — 이미
`oxilean_parse::Decl` 을 consume 하던 downstream 코드는
rewrite 가 아니라 translator (`leo4-oxilean-build` 내
`leo4_translate` module) 만 새로 얻음.

integration test (`tests/oxilean_cross_check.rs`) 로
공유 corpus 에서 두 parser 동시 실행 — `oxilean-parse`
가 받는 모든 입력은 `leo4-lean4-parse` 도 받아야 하며
decl 수 + 이름 + kind tag 가 일치해야 함.

`leo4-oxilean-build` 의 `[features]` 를 뒤집어
`leo4-parser` 를 `default` 에 넣음. oxilean-parse-direct
는 `TranslateError::Unsupported` 발생 시 fallback
(oxilean 대응이 없는 `Dsl`, `HashCommand`,
`DefinitionByArms` 등).

== OX5-oxi: elab env bootstrap

rust-transpile pipeline 의 `transpile_source_to_unit`
는 `Environment::new()` 로
`oxilean_elab::elaborate_decl(&env, &decl)` 호출. 그래서
파싱 성공한 `def x : UInt64 := 0` 도
`NameNotFound("UInt64")` 로 실패. fix:
`leo4-oxilean-build` 의 `leo4_env_bootstrap` module 이
`oxilean_kernel::init_builtin_env` (Bool / Unit /
Empty / Nat / String / Eq / Prod / List + 공리 + Nat
arithmetic) 호출 후 leo4 가 필요로 하는 boundary
primitive (OxiLean 이 ship 하지 않는 `UInt8..128`,
`Int8..128`, `Float32`, `Float64`, `Char`) 를
`Declaration::Axiom { ty: Sort 1, … }` 로 augment.

augmentation 목록은 `LEO4_PRIMITIVE_TYPES: &[&str]` 에
single-source. OxiLean upstream 이 augmentation 이름
중 하나를 ship 하기 시작하면 (silent
`DuplicateDeclaration` 유발) 시끄럽게 fail 하도록
`oxilean_kernel::builtin::all_builtin_names()` 와
교차 점검하는 regression-guard test 추가.

== OX5-msl: 확인된 no-op

mslean4 backend (lean.h + libleanshared) 로 leo4 를
빌드하는 경우 OX5 문제 재발 안 함 — lake plugin 이
Lean 자체 elaborator 를 `import Lean` 컨텍스트에서
돌리므로 `UInt64` / `+` 는 construction-by-default 로
visible. split 의 mslean4 half 는 문서적 artefact 일
뿐 code 작업 아님. code audit
(`grep -rn 'Environment::new\|elaborate_decl'`) 으로
모든 call site 가 `sibling/leo4-oxilean-build` 안에
있음을 확인.

== Post-OX6 CLI refactor

leo4 CLI 의 `create` / `init` 의 `--impl <kind>` flag
가 per-(sub)crate `leo4.toml` 파일로 이전. `Leo4Config`
parser (TOML, `[[impl]]` arrays-of-tables) 구축,
disjoint output path validation. `create` 에
`--subcrate` 추가 — 위로 올라가며 가장 가까운
`[workspace]` Cargo.toml 을 찾아 새 crate 를 그
`members` 배열에 등록 (inline + multi-line 둘 다,
idempotent). `init` 은 3-way precedence 획득:
기존 `leo4.toml` → 손대지 않음; legacy `.leo4-impl`
marker → migrate + delete; 둘 다 없음 → default
`[[impl]] kind = "mslean4"`. `run` 은 4-way precedence
로 impl 해석: `leo4.toml + --impl` (selector) → 첫
`[[impl]]` → legacy marker → hard error.

== C5: musl Tier 1+ (no-mslean4-no-lake paths)

host 가 glibc 인데 OxiLean 전용 transpile path 용
static musl binary 를 ship 하고 싶다면, leo4 source 변경
0. audit verified: 14 workspace crates 가
`--target x86_64-unknown-linux-musl` 에서 out-of-box
clean; 2 (`leo4-rust-bridge`, `leo4-wasm`) 는 host
musl C toolchain (`musl-clang` 또는 `musl-gcc`) 필요.
Arch 의 `musl-clang` wrapper 는 packaging quirk —
`-nostdinc` 를 패스하고 clang 의 freestanding-header
path 를 복원하지 않음. `leo4-rust-bridge` 의 `build.rs`
가 wrapper 자동 감지 후
`-isystem $(clang -print-resource-dir)/include` 추가
→ `<stdatomic.h>` 해결. 다른 toolchain 에서는 no-op.

== Leo4.Platform Lean layer

`lake/Leo4/Leo4/Build.lean` 내 OS-PORTABILITY ledger
3 개 항목 (`.so` 확장자 hardcode, `-Wl,-rpath` 도처에,
`-shared` flag) 이 새 `lake/Leo4/Leo4/Platform.lean`
module 로 이동 — `dynlibExt`, `dynlibPrefix`,
`isPlatformDynlib`, `stemOfDynlib`, `linkRpath?`,
`defaultShimSuffix`. `Build.lean` 의 `collectLibDir`
와 `linkShared` 는 hardcode literal 대신 helper 를
consume. OS-PORTABILITY 정책: 새 per-OS branch 는
이 module 안으로.

== Windows IPC worker side

`leo4-rust-worker` 의 `open_ipc_channel` Windows branch
는
`"Windows named-pipe IPC not yet implemented"` 를
반환하는 stub 이었음. 채우기:
`std::fs::OpenOptions::new().read(true).write(true).open(pipe_path)`
(내부적으로 `CreateFileW`) 가 dispatcher 의
`CreateNamedPipeA` / `ConnectNamedPipe` 의 client-side
counterpart. dispatcher 가 OS 에 pipe 이름을 register
하기 전에 worker 가 spawn 되는 좁은 race 에 대비해
10x linear backoff retry.
