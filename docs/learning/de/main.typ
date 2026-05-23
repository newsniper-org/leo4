#import "../../template/leo4-book.typ": book
#show: book.with(
  title: "leo4 — Lehrmaterial",
  subtitle: "Deutsche Ausgabe",
  author: "윤병익 (leo4-Projekt)",
  lang: "de",
)

= Einleitung

`leo4` ist eine Lean-4-↔-Rust-Interop-Bibliothek, die die
Rust-Seite absichtlich *nicht* an eine bestimmte
Lean-Toolchain-Version bindet. Der Vorgänger `leo3` kompilierte
direkt gegen `lean.h` --- und brach jedes Mal, wenn sich Leans
internes Layout änderte. `leo4` kapselt das gesamte Wissen über
das Lean-ABI in einem zur Bauzeit erzeugten C-Shim und legt der
Rust-Crate nur ein stabiles kanonisches ABI offen.

Das Ergebnis: die Rust-Crate folgt dem IDL (einer kleinen
WIT-Obermenge-Schemasprache), nicht der Lean-Toolchain.
Lean-Upgrades rotieren den Shim, aber nicht das Rust-Binary.

Dieses Lehrmaterial geht leo4 so durch, wie eine erfahrene
Ingenieurin es lernen würde: an der Oberfläche beginnen (was
schreibt eine Anwenderin?), dann Schicht für Schicht abtragen
(wie wird das über die Grenze transportiert?) und am Ende die
Entwurfsentscheidungen ansehen, die die Architektur geformt
haben.

== Zielgruppe

Sie sind mit mindestens einem von Lean 4 oder Rust vertraut und
bereit, genug von der anderen Seite zu lernen, um den
Grenzübertritt zu verfolgen. Wir setzen voraus:

- Rust-Grundlagen: `Cargo.toml`, Traits, Lifetimes (`'a`),
  Procedural Macros auf Nutzungsebene (Sie müssen kein Macro
  schreiben, nur verstehen, was es erzeugt).
- Lean-4-Grundlagen: `def`, `structure`, `inductive`,
  Typklassen (`class` / `instance`) und die Vorstellung, dass
  ein Lean-Ausdruck sowohl einen abstrakten Typ als auch eine
  kompilierte Laufzeitrepräsentation hat.
- Ein vages Gefühl für Foreign Function Interfaces (FFI) auf
  C-ABI-Ebene --- Zeiger, sizeof, Aufrufkonventionen.

Sie müssen das WebAssembly-Component-Modell oder WASIp3 nicht
kennen, außer für die jeweils zuständigen Kapitel.

= Die Dreißig-Sekunden-Tour

Der einfachste leo4-Anwendungsfall sieht so aus. Auf der
Lean-Seite:

```lean
import Leo4

namespace Sample

@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b
```

Auf der Rust-Seite:

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

`@[leo4_export]` teilt dem Lake-Plugin mit, „diese Deklaration
überquert die Grenze". `leo4::import!` auf der Rust-Seite liest
die vom Plugin erzeugte Mangling-Tabelle und synthetisiert einen
Rust-Wrapper, der die Argumente nach leo4s kanonischem ABI
kodiert, den passenden C-Shim-Eintrittspunkt aufruft, den
Rückgabewert dekodiert und das Ergebnis in ein `Result`
einpackt.

= Architektur-Überblick

leo4 hat sechs bewegliche Teile. Zu wissen, was jeder Teil
besitzt, ist die halbe Miete für das mentale Modell.

== Das Lake-Plugin (`lake/Leo4Plugin/`)

Ein Lean-Executable, das das Paket der Anwenderin lädt, jede
`@[leo4_export]`-Definition besucht und pro Build vier
Artefakte erzeugt:

#table(
  columns: (auto, 1fr),
  table.header[*Datei*][*Zweck*],
  [`<pkg>.leo4-schema`],
  [Kanonische IDL-Form: Typdeklarationen + Funktionssignaturen
   in stabilem Textformat. Eingabe für den Schema-Hash.],
  [`<pkg>.leo4-mangling`],
  [JSON-Tabelle, die logische Funktionsnamen + Mangling pro
   Argumenttyp auf das eindeutige C-Symbol abbildet, das der
   Shim aufruft.],
  [`<pkg>.leo4-handshake`],
  [Der Schema-Hash + Lean-Toolchain-Identifikator + Liste
   exportierter Interfaces. Der Rust-Loader liest ihn zur
   `Lean::open`-Zeit.],
  [`<pkg>.leo4-shim.{c,so}`],
  [Erzeugter C-Quelltext, kompiliert zu einer Shared Library;
   pro Export ein `leo4_call_<mangled>`-Eintrittspunkt. Die
   einzige Stelle im System, an der `lean/lean.h`
   eingebunden wird.],
)

Das Plugin schreibt außerdem eine
`<pkg>.leo4-exports.lean`-Datei: ein Lean-Wrapper-Modul, gegen
das der Shim linkt und das die Exporte der Anwenderin in einer
bekannten Namens-Surface umhüllt
(`@[export leo4_lean__<mangled>]`).

== `leo4-abi` (kanonisches ABI-Marshalling)

Eine Rust-Crate, die `lake/Leo4/Leo4/Marshal.lean` und
`Builtins.lean` byteweise spiegelt. Beide Seiten implementieren
das `LeanMarshal`-Trait / die Typklasse; die Testsuite
(`tests/conformance/`) verifiziert für jeden unterstützten Typ,
dass der Lean-Encoder und der Rust-Encoder byteweise identische
Ausgaben erzeugen.

== `leo4-mslean4` (Loader + Dispatch)

Eine Rust-Crate, die `Lean::open`, `Arena<'a>` und
`LeanRef<'a, T>` bereitstellt. Der Loader nutzt `libloading`,
um die `.so` des Shims hochzufahren, initialisiert die
Lean-Laufzeit einmal pro Prozess, prüft den Schema-Hash gegen
die Rust-interne Konstante, führt das `initialize_*`-Symbol
des Wrapper-Moduls aus und verteilt anschließend
`leo4_call_<mangled>`-Aufrufe über einen pro Name geführten
Funktionszeiger-Cache.

== `leo4-macros` (`leo4::import!`, `#[derive(LeanMarshal)]`)

Procedural Macros. `leo4::import!` parst einen
extern-Block-artigen Block aus `fn`-Signaturen, sucht sie in
der Mangling-JSON, die das Build-Skript über `OUT_DIR`
bereitstellt, und gibt Rust-Wrapper-Funktionen aus.
`#[derive(LeanMarshal)]` synthetisiert Encode / Decode für
Anwendertypen, die einer der vier kanonischen ABI-Shapes
entsprechen (Record, all-unit Enum, Mixed-Payload-Variante,
Single-`u64`-Resource).

== `leo4`-Fassade

Eine dünne Re-export-Crate. Die Anwenderin fügt eine Zeile
hinzu: `leo4 = { workspace = true }`. Alles andere --- `Lean`,
`LeanRef`, `LeanError`, `import!`, `LeanMarshal` --- lebt unter
`leo4::*`.

== `leo4-build`

Ein `build.rs`-Helper. Eine Zeile in der `build.rs` der
Konsumenten-Crate:

```rust
fn main() {
    leo4_build::wire("path/to/<pkg>/.lake/build/leo4").unwrap();
}
```

emittiert die richtigen `cargo:rustc-link-search`,
`cargo:rerun-if-changed=` und `env!("LEO4_SHIM_SO")` /
`env!("LEO4_HANDSHAKE_FILE")`-Konstanten, die der Macro und der
Loader erwarten.

= Das IDL --- eine WIT-Obermenge

leo4s IDL ist die kanonische Schnittstelle auf Typenebene
zwischen den beiden Seiten. Es geht von WIT des
WebAssembly-Component-Modells aus und ergänzt die wenigen
Konstrukte, die Leans abhängige Typen brauchen, um an der
Grenze unterzukommen.

Die Grammatik lebt in `SPEC/idl-grammar.ebnf`. Die wichtigsten
Erweiterungen gegenüber WIT sind:

#table(
  columns: (auto, 1fr),
  table.header[*Konstrukt*][*Warum*],
  [`generic_params` auf Nominal-Deklarationen],
  [Leans nutzerdefinierte Typen sind generisch. `record
   Pair<α, β>` parst als Record mit zwei Typparametern; jede
   Instanziierung erhält ihren eigenen Mangled Name.],
  [`Self` / `Self<…>`-Selbstreferenzen],
  [Varianten wie `Tree { leaf, node(Self, Self) }` rekursieren
   durch die umgebende Deklaration. Die Mangling-Regel
   (`SPEC/mangling.md` §„Self and Self<…>") emittiert ein
   kurzes Token statt des vollen FQN.],
  [`mutual { … }`-Cluster + `Cyc<i>`],
  [Phase 6: gegenseitige Rekursion zwischen zwei
   nominalen Typen. `Cyc<i>` referenziert das `i`-te Mitglied
   des Clusters.],
  [`constraint <name> = <body>`-Deklarationen],
  [Constraints wie `oneof { … }` pinnen das Admit-Set einer
   Generic. Nur auf Typenebene; erreichen die Leitung nie.],
  [`bigint` / `bignat`],
  [Ganzzahlen beliebiger Genauigkeit. Drahtform ist Sign +
   Limbs (SPEC/canonical-abi.md §6).],
  [`external <fqn>`],
  [Phase 8: ein nominaler Typ, dessen Drahtformat in einer
   eigenen `LeanMarshal`-Instanz lebt statt in
   Feldwise-Codegen. Verwendet für `Rat` und jeden anderen
   Mathlib-geformten Typ mit beweistragenden Feldern.],
)

WIT-seitig senkt `leo4c lower` jedes IDL-Fragment in eine
WIT-Datei. Die WIT-Ausgabe lässt sich mit `wasm-tools` und
`wit-bindgen` für Component-Model-Deployments verwenden.

= Das kanonische ABI --- Bytes auf der Leitung

`SPEC/canonical-abi.md` ist normativ. Rust- und Lean-Encoder
müssen für denselben logischen Wert identische Bytes erzeugen;
der Conformance-Harness (`tests/conformance/run.sh`) verankert
dies über 29 Fixtures.

Stichpunkte, falls Sie nicht die ganze Spec lesen möchten:

- Ganzzahlen sind Little-Endian; signed und unsigned teilen
  sich dasselbe Bitmuster (signed im Zweierkomplement).
- Strings sind `u32 len + UTF-8-Bytes`.
- Listen sind `u32 len + N Element-Kodierungen`.
- Optionen sind `u8 disc (0=none, 1=some) + Payload`.
- Results sind `u8 disc (0=ok, 1=err) + Payload`.
- Varianten nutzen `u32 LE disc + Payload` (SPEC §9; in Commit
  b2aa323 vom 2026-05-20 auf u32 festgelegt --- die SPEC
  erlaubte u8 für ≤256 Fälle, beide Encoder emittieren jetzt
  4 Bytes).
- Records hängen Feldkodierungen in Deklarationsreihenfolge
  aneinander.
- Resources sind ein opaker `u64`-Handle.
- `bigint` / `bignat` sind längenpräfixierte Limb-Arrays plus
  Vorzeichen.

Der Shim-Emitter und das Rust-Derive-Macro erzeugen Code, der
diesen Formaten folgt. Das `walkUserDecl` des Plugins entdeckt
Anwendertypen und synthetisiert das passende Encode / Decode
ohne handgeschriebenen Code pro Typ.

= Mangling --- Namenskonventionen

`SPEC/mangling.md` legt die C-Symbolnamen fest. Die Vollform
lautet

```
leo4__<pkg_seg>__<iface>__<fname>__<arg_mangles>__h<schema_hash>
```

Jedes Teil ist ASCII-sicher; Punkte in FQNs werden zu
Unterstrichen; generische Argumente entfalten sich in
Segmente pro Instanziierung. Der Schema-Hash ist `FNV-1a-64`
über den normalisierten IDL-Text, gerendert als
13-Zeichen-base32lc. Ändert sich irgendeine Export-Signatur,
rotiert der Hash und damit jeder Mangled Name im Paket ---
ein veraltetes Rust-Binary, das gegen einen frischen Shim
linkt, scheitert daher beim Linken.

Die Hash-Konstruktion ist in `SPEC/mangling.md` §3 beschrieben.
Beide Implementierungen (Rust-`crates/schema-idl` und
Lean-`lake/Leo4Plugin/Leo4Plugin/Mangling.lean`) müssen sie
identisch berechnen; `tests/mangling/` pinnt 67+ Namen
byteidentisch zwischen den beiden.

= Typsystem auf der Rust-Seite

Die Grenze nutzt zwei Haupt-Traits:

- `LeanMarshal` --- kanonisches ABI-Encode / Decode.
  Implementiert für alle primitiven Typen, Composites
  (`Vec<T>`, `Option<T>`, `Result<T,E>`, Tuples) und über
  `#[derive]` für Anwender-Records, Enums, Varianten und
  Resources.
- `LeanType` --- Typsystem-Marker, der mit der Schema-Schicht
  verbindet. Die meisten Anwenderinnen berühren das nicht
  direkt; `#[derive]` und die Macros erledigen es.

Außerdem gibt es `LeanResource` für opake Handles. Ein Typ
kann nicht gleichzeitig `LeanMarshal` und `LeanResource` sein
--- das Plugin erzwingt das.

Die Lean-Seite spiegelt das Trait / die Typklasse mit
`class Leo4.LeanMarshal` und dem zugehörigen `deriving
LeanMarshal`-Handler. Die beiden Bytestreams müssen
übereinstimmen; der Conformance-Harness ist die
cross-implementierungs-Prüfung.

= Phasen-Leiter --- wann jede Fähigkeit gelandet ist

Die leo4-Entwicklung folgt einer Phasen-Leiter. Zu wissen,
aus welcher Phase ein Feature stammt, hilft beim Lesen von
Commit-Nachrichten.

#table(
  columns: (auto, 1fr),
  table.header[*Phase*][*Was gelandet ist*],
  [0], [Lake-Hook-Spike --- fand den richtigen
        Plugin-Integrationspunkt (`lean_exe` nach `lake build`
        aufgerufen, kein `recBuildLean`-Hook).],
  [1], [Lean-Laufzeitbibliothek + Lake-Plugin;
        Admit-Set-Algorithmus.],
  [2], [Rust-`leo4-idl` + cross-impl Mangling-Conformance.],
  [3], [WIT-Lowering-Pass + `wasm-tools`-Validierung.],
  [4], [Kanonische ABI-Conformance-Harness, `bignat` /
        `bigint`.],
  [5], [C-Shim-Synthese + `leo4-mslean4` + `leo4-macros` +
        `examples/01-hello`, `examples/02-roundtrip`.
        End-to-End-Pipeline.],
  [6], [Gegenseitige Rekursion zwischen nominalen Typen
        (`mutual { … }`-IDL-Block, `Cyc<i>`,
        `examples/04-mutual-ast`).],
  [7], [Asynchrones `io<T>`-Lowering. Parser desugart
        `future<T>` / `stream<T>`; Shim umhüllt `IO α`-Lean-
        Wrapper in `lean_io_result_*`. WASIp3-Schwester-Projekt
        für die wasm-async-Surface.],
  [8], [Mathlib-kompatible Untermenge: `LeanRat`, `LeanU128` /
        `LeanI128`, `LeanComplexF{32,64}x2`, `LeanF16` /
        `LeanBF16` / `LeanF128` (nightly), Mathlib-Bridges mit
        IEEE-754-RTNE-Rundung.],
)

= Abschluss

Dieses Lehrmaterial ist ein Ausgangspunkt. Das begleitende
`implement-from-scratch`-Handbuch geht den nächsten Schritt:
Es führt durch das *Bauen* jeder Schicht von leo4 selbst, in
der Reihenfolge der ursprünglichen Phasen.

Für den täglichen Bezug:

- `SPEC/*.md` ist normativ; ist etwas unklar, in der Spec
  nachsehen.
- `CHANGELOG.md` listet die Wirkung jedes Commits mit
  Begründung.
- `ROADMAP.md` beschreibt die Phasen-Leiter.
- `LEO4-DESIGN.md` erfasst jede architektonische Entscheidung
  und die Begründung dazu.

Das Repository ist die einzige Wahrheitsquelle. Alles andere
ist Kommentar.
