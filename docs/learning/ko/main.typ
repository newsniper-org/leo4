#import "../../template/leo4-book.typ": book
#show: book.with(
  title: "leo4 — 학습 자료",
  subtitle: "한국어판",
  author: "윤병익 (leo4 프로젝트)",
  lang: "ko",
)

= 들어가며

`leo4` 는 Lean 4 와 Rust 사이의 상호 운용성 라이브러리이며,
의도적으로 Rust 측을 특정 Lean 툴체인 버전에 묶지 *않습니다*.
이전 작업이었던 `leo3` 는 `lean.h` 를 직접 컴파일했고 ---
Lean 의 내부 레이아웃이 바뀔 때마다 깨졌습니다 --- `leo4` 는
모든 Lean ABI 지식을 빌드 시점에 생성되는 C shim 에 격리하고,
Rust crate 에는 안정적인 canonical ABI 만 노출합니다.

결과적으로 Rust crate 는 IDL (WIT 의 상위호환인 작은 스키마 언어)
을 추적하며, Lean 툴체인을 추적하지 않습니다. Lean 업그레이드는
shim 만 회전시키고 Rust 바이너리는 건드리지 않습니다.

이 학습 자료는 시니어 엔지니어가 leo4 를 익히는 순서로 안내합니다:
표면 (사용자가 무엇을 쓰는가?) 에서 시작해서 한 층씩 벗기고
(어떻게 경계를 넘어 전송되는가?), 마지막으로 아키텍처를 만든
설계 결정들을 살펴봅니다.

== 대상 독자

Lean 4 또는 Rust 둘 중 적어도 하나에 익숙하고, 경계 횡단을
따라가기에 필요한 만큼만 다른 쪽을 익힐 의지가 있어야 합니다.
가정하는 배경:

- 기초 Rust: `Cargo.toml`, trait, lifetime (`'a`), procedural
  macro 의 사용자 수준 이해 (직접 작성할 필요는 없고, 생성되는
  코드의 의미만 알면 됩니다).
- 기초 Lean 4: `def`, `structure`, `inductive`, 타입 클래스
  (`class` / `instance`), 그리고 Lean 표현식이 추상 타입과
  컴파일된 런타임 표현 양쪽을 가진다는 사고방식.
- C ABI 수준의 FFI 에 대한 막연한 감각 --- 포인터, sizeof,
  호출 규약.

wasm Component Model 또는 WASIp3 는 해당 백엔드 챕터를 읽을 때만
필요합니다.

= 30초 투어

가장 간단한 leo4 사용 사례. Lean 측:

```lean
import Leo4

namespace Sample

@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b
```

Rust 측:

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

`@[leo4_export]` 는 Lake 플러그인에게 "이 정의가 경계를 넘는다"
고 알립니다. Rust 측의 `leo4::import!` 는 플러그인이 만든
mangling 테이블을 읽고, 인수를 leo4 의 canonical ABI 로
인코딩하고, 일치하는 C shim 진입점을 호출하고, 반환값을
디코딩하여 `Result` 로 감싸는 Rust wrapper 를 합성합니다.

= 아키텍처 개요

leo4 는 여섯 개의 움직이는 부품으로 구성됩니다. 각각의 책임을
알면 정신 모델의 절반이 갖춰집니다.

== Lake 플러그인 (`lake/Leo4Plugin/`)

사용자 패키지를 로드하고, 모든 `@[leo4_export]` 정의를 순회하며,
빌드 당 네 개의 산출물을 생성하는 Lean 실행 파일:

#table(
  columns: (auto, 1fr),
  table.header[*파일*][*용도*],
  [`<pkg>.leo4-schema`],
  [정규 IDL 형식: 타입 선언 + 함수 시그너처를 안정적인 텍스트
   형식으로. 스키마 해시의 입력.],
  [`<pkg>.leo4-mangling`],
  [논리적 함수명 + 인수 타입별 mangling 을 shim 이 호출하는
   고유 C 심볼로 매핑하는 JSON 테이블.],
  [`<pkg>.leo4-handshake`],
  [스키마 해시 + Lean 툴체인 식별자 + 내보낸 인터페이스 목록.
   Rust 로더가 `Lean::open` 시점에 읽습니다.],
  [`<pkg>.leo4-shim.{c,so}`],
  [생성된 C 소스를 공유 라이브러리로 컴파일. export 당
   `leo4_call_<mangled>` 진입점 하나씩. 시스템에서 유일하게
   `lean/lean.h` 를 `#include` 하는 자리.],
)

플러그인은 또한 `<pkg>.leo4-exports.lean` 을 작성합니다 ---
shim 이 링크하는 Lean wrapper 모듈로, 사용자 export 를 알려진
이름 surface 로 감싸는 `@[export leo4_lean__<mangled>]`
선언들을 제공합니다.

== `leo4-abi` (canonical-ABI marshalling)

`lake/Leo4/Leo4/Marshal.lean` 과 `Builtins.lean` 을 바이트 단위로
mirror 하는 Rust crate. 양 측이 `LeanMarshal` trait / 타입클래스를
구현하며, 테스트 스위트 (`tests/conformance/`) 가 지원되는 모든
타입에 대해 Lean encoder 와 Rust encoder 가 바이트 단위로 동일한
출력을 생성함을 검증합니다.

== `leo4-mslean4` (loader + dispatch)

`Lean::open`, `Arena<'a>`, `LeanRef<'a, T>` 를 제공하는 Rust
crate. 로더는 `libloading` 으로 shim 의 `.so` 를 가져오고, 프로세스
당 한 번 Lean 런타임을 초기화하고, 스키마 해시를 Rust 내장 상수와
대조하고, wrapper 모듈의 `initialize_*` 심볼을 실행한 뒤,
이름별 함수 포인터 캐시를 통해 `leo4_call_<mangled>` 호출을
디스패치합니다.

== `leo4-macros` (`leo4::import!`, `#[derive(LeanMarshal)]`)

Procedural macro. `leo4::import!` 는 `fn` 시그너처의 extern
스타일 블록을 파싱하고, build script 가 `OUT_DIR` 로 surface 한
mangling JSON 에서 찾아 Rust wrapper 함수를 emit 합니다.
`#[derive(LeanMarshal)]` 은 네 가지 canonical-ABI shape (record,
all-unit enum, mixed-payload variant, single-`u64` resource) 에
맞는 사용자 타입의 encode/decode 를 합성합니다.

== `leo4` façade

얇은 re-export crate. 사용자는 한 줄만 추가하면 됩니다:
`leo4 = { workspace = true }`. 그 외 모든 것 --- `Lean`,
`LeanRef`, `LeanError`, `import!`, `LeanMarshal` --- 은
`leo4::*` 에 있습니다.

== `leo4-build`

`build.rs` 헬퍼. 소비자 crate 의 `build.rs` 한 줄:

```rust
fn main() {
    leo4_build::wire("path/to/<pkg>/.lake/build/leo4").unwrap();
}
```

이 한 줄이 매크로와 로더가 기대하는
`cargo:rustc-link-search`, `cargo:rerun-if-changed=`,
`env!("LEO4_SHIM_SO")` / `env!("LEO4_HANDSHAKE_FILE")` 상수들을
emit 합니다.

= IDL --- WIT 의 상위호환

leo4 의 IDL 은 양 측 간의 정형 타입-레벨 인터페이스입니다.
WebAssembly Component Model 의 WIT 에서 출발해서, Lean 의 의존
타입이 경계에 맞도록 필요한 작은 구성요소들을 추가했습니다.

문법은 `SPEC/idl-grammar.ebnf` 에 있습니다. WIT 대비 주요
확장:

#table(
  columns: (auto, 1fr),
  table.header[*구성요소*][*이유*],
  [nominal 선언의 `generic_params`],
  [Lean 의 사용자 정의 타입은 제네릭. `record Pair<α, β>` 는
   두 타입 매개변수를 가진 record 로 파싱되며, 각 인스턴스화는
   자체 mangled name 을 얻습니다.],
  [`Self` / `Self<…>` 자기 참조],
  [`Tree { leaf, node(Self, Self) }` 같은 variant 는 enclosing
   decl 을 통해 재귀합니다. Mangling 규칙
   (`SPEC/mangling.md` §"Self and Self<…>") 은 전체 FQN 대신
   짧은 토큰을 emit 합니다.],
  [`mutual { … }` 클러스터 + `Cyc<i>`],
  [Phase 6: 두 nominal 타입 간의 상호 재귀. `Cyc<i>` 는 클러스터의
   `i` 번째 멤버를 참조합니다.],
  [`constraint <name> = <body>` 선언],
  [`oneof { … }` 같은 제약이 generic 의 admit-set 을 고정합니다.
   타입 레벨 전용; wire 에는 절대 도달하지 않습니다.],
  [`bigint` / `bignat`],
  [임의 정밀도 정수. Wire 형식은 부호 + limb
   (SPEC/canonical-abi.md §6).],
  [`external <fqn>`],
  [Phase 8: 필드별 codegen 대신 custom `LeanMarshal` 인스턴스에
   wire 형식이 살아있는 nominal 타입. proof-carrying 필드를 가진
   `Rat` 와 같은 Mathlib-shape 타입에 사용.],
)

WIT 측에서는 `leo4c lower` 가 각 IDL 단편을 WIT 파일로 내립니다.
WIT 출력은 `wasm-tools` 와 `wit-bindgen` 으로 Component Model
배포를 위해 소비할 수 있습니다.

= Canonical ABI --- wire 상의 바이트

`SPEC/canonical-abi.md` 가 규범입니다. Rust 와 Lean encoder 는
같은 논리 값에 대해 같은 바이트를 생성해야 하며, conformance
harness (`tests/conformance/run.sh`) 가 29 개 fixture 에서 이를
고정합니다.

스펙을 다 읽고 싶지 않다면, 요점:

- 정수는 little-endian; 부호 있는 / 없는 정수는 같은 비트 패턴을
  공유 (부호 있는 정수는 2 의 보수).
- 문자열은 `u32 len + utf-8 바이트`.
- 리스트는 `u32 len + N 요소 인코딩`.
- Option 은 `u8 disc (0=none, 1=some) + payload`.
- Result 는 `u8 disc (0=ok, 1=err) + payload`.
- Variant 는 `u32 LE disc + payload` (SPEC §9; 우리는 2026-05-20
  commit b2aa323 에서 u32 로 못박았습니다 --- SPEC 가 ≤256 case
  에 u8 을 허용하지만 양 encoder 모두 4 바이트 emit).
- Record 는 필드 인코딩을 선언 순서대로 연결.
- Resource 는 불투명 `u64` 핸들.
- `bigint` / `bignat` 은 길이 접두사 limb 배열 + 부호.

shim emitter 와 Rust derive macro 가 이 형식을 따르는 코드를
생성합니다. 플러그인의 `walkUserDecl` 이 사용자 타입을 발견하고,
타입별 수작업 없이 일치하는 encode/decode 를 합성합니다.

= Mangling --- 명명 규칙

`SPEC/mangling.md` 가 C 심볼 이름을 정의합니다. 전체 형식은:

```
leo4__<pkg_seg>__<iface>__<fname>__<arg_mangles>__h<schema_hash>
```

각 조각은 ASCII-안전; FQN 의 점은 underscore 로; generic 인수는
인스턴스화별 세그먼트로 확장됩니다. 스키마 해시는 정규화된 IDL
텍스트에 대한 `FNV-1a-64` 이며, 13 자 base32lc 로 렌더됩니다.
어떤 export 의 시그너처가 바뀌면 해시가 회전하고 따라서 패키지의
모든 mangled name 이 회전합니다 --- 그래서 fresh shim 과 링크하는
오래된 Rust 바이너리는 링크 시점에 실패합니다.

해시 구성은 `SPEC/mangling.md` §3 에 문서화되어 있습니다. 두
구현 (Rust `crates/schema-idl` 과 Lean
`lake/Leo4Plugin/Leo4Plugin/Mangling.lean`) 이 동일하게 계산해야
하며, `tests/mangling/` 이 양 측 67+ 이름을 바이트 단위로 동일
하게 고정합니다.

= Rust 측 타입 시스템

경계는 두 개의 주요 trait 를 사용합니다:

- `LeanMarshal` --- canonical-ABI encode/decode. 모든 primitive
  타입, composite (`Vec<T>`, `Option<T>`, `Result<T,E>`, tuple)
  에 구현되어 있고, `#[derive]` 를 통해 사용자 record, enum,
  variant, resource 에도.
- `LeanType` --- 스키마 layer 에 연결되는 타입-시스템 marker.
  대부분 사용자는 직접 다루지 않습니다; `#[derive]` 와 macro
  가 처리.

불투명 핸들을 위한 `LeanResource` 도 있습니다. 한 타입이 동시에
`LeanMarshal` 과 `LeanResource` 일 수는 없습니다 --- 플러그인이
이를 강제.

Lean 측은 `class Leo4.LeanMarshal` 과 일치하는 `deriving
LeanMarshal` 핸들러로 trait/타입클래스를 mirror 합니다. 두 바이트
스트림은 일치해야 하며; conformance harness 가 cross-impl 검사.

= Phase 사다리 --- 각 기능이 land 한 시점

leo4 개발은 phase 사다리를 따릅니다. 각 기능이 어느 phase 에서
왔는지 알면 커밋 메시지를 읽을 때 도움이 됩니다.

#table(
  columns: (auto, 1fr),
  table.header[*Phase*][*Landed*],
  [0], [Lake hook spike --- 적절한 플러그인 통합 지점
        (`lake build` 후 호출되는 `lean_exe`, `recBuildLean`
        훅이 아님) 발견.],
  [1], [Lean 런타임 라이브러리 + Lake 플러그인; admit-set 알고리듬.],
  [2], [Rust `leo4-idl` + cross-impl mangling 컴포넌스.],
  [3], [WIT lowering 패스 + `wasm-tools` 검증.],
  [4], [Canonical-ABI 컴포넌스 harness, `bignat` / `bigint`.],
  [5], [C shim 합성 + `leo4-mslean4` + `leo4-macros` +
        `examples/01-hello`, `examples/02-roundtrip`.
        엔드 투 엔드 파이프라인.],
  [6], [nominal 타입 간 mutual recursion (`mutual { … }` IDL
        블록, `Cyc<i>`, `examples/04-mutual-ast`).],
  [7], [비동기 `io<T>` lowering. 파서가 `future<T>` /
        `stream<T>` 를 desugar; shim 이 `IO α` Lean wrapper 를
        `lean_io_result_*` 로 감쌈. wasm-async surface 를 위한
        WASIp3 sibling project.],
  [8], [Mathlib 호환 subset: `LeanRat`, `LeanU128` /
        `LeanI128`, `LeanComplexF{32,64}x2`, `LeanF16` /
        `LeanBF16` / `LeanF128` (nightly), IEEE-754 RTNE
        rounding 기반 Mathlib bridge.],
)

= 맺음말

이 학습 자료는 시작점입니다. 동반 `implement-from-scratch`
가이드북은 다음 걸음을 짚습니다: leo4 의 각 layer 를 직접
*구축* 하는 과정을, 원래 phase 들이 land 한 순서대로 안내.

일상 참조용:

- `SPEC/*.md` 는 규범; 무엇이 불분명하면 spec 을 확인.
- `CHANGELOG.md` 는 모든 commit 의 효과와 근거를 나열.
- `ROADMAP.md` 는 phase 사다리를 설명.
- `LEO4-DESIGN.md` 는 모든 아키텍처 결정과 그 근거를 담음.

리포지토리가 단일 source of truth. 나머지는 모두 주석.
