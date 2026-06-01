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

= Update — 2026-05-24

Zusammengefasste Übersicht der Änderungen nach dem
v0.1.0-Cut.

- *OX6 — PEG-basierter Lean-4-Parser (abgeschlossen)*.
  `sibling/leo4-lean4-parse` ersetzt die textuelle
  Pre-Rewrite-Kette (OX3/OX4) in
  `leo4-oxilean-build`. AST-Formen spiegeln
  `oxilean-parse` v0.1.2 wider; eine strikte
  Obermenge dessen akzeptierter Surface. Lean-4-Formen,
  die die rust-transpile-Pipeline zuvor ablehnte
  (Block- / Doc-Kommentare, Unicode-Operatoren
  `≤ ≥ ≠ × ÷ ∈`, `if let` / `match h : e with` /
  Pattern Guards, anonyme Fn `(· + 1)`,
  DSL-Deklarationen `notation` / `macro_rules` /
  `syntax` / `elab`, equationales `def | pat =>`, …)
  elaborieren jetzt.

- *OX5 — Elab-Env-Bootstrap (abgeschlossen)*. Der
  Elaborator der rust-transpile-Pfade scheitert
  nicht mehr an `NameNotFound("UInt64")`. Der reine
  OxiLean-Nutzer installiert *nichts* über
  `leo4-oxilean-build` hinaus —
  `oxilean_kernel::init_builtin_env` plus eine
  kleine leo4-seitige Ergänzung (Größen-Integer,
  Floats, Char) füllt die Env in-process.

- *Post-OX6 CLI-Refactor — `leo4.toml`*. `leo4 create`
  und `leo4 init` haben das Flag `--impl <kind>`
  abgeschafft. Jedes neu erzeugte Projekt enthält
  eine `leo4.toml`, die Runtime-Implementierungen
  deklariert. Mehrere `[[impl]]`-Einträge sind
  erlaubt; disjunkte Output-Pfade werden beim Parsen
  erzwungen. `leo4 create --subcrate` registriert das
  neue Crate im `members`-Array des umgebenden
  Workspaces. `leo4 init` migriert das alte
  `.leo4-impl`-Marker automatisch.

- *C5 — musl Tier 1+ (v1.0-RC-Pflicht)*. Die Linux-
  Ziele `*-linux-musl*` werden für Pfade ohne
  `leo4-mslean4`-Runtime und ohne `lake`-Aufruf
  unterstützt (rust-transpile end-to-end,
  scaffold-only CLI-Befehle, alle pure-Rust-Crates).
  Im leo4-Quelltext sind keine per-target-Zweige
  nötig. Eine Host-Eigenheit (Archs `musl-clang`-
  Wrapper, dem freistehende Header fehlen) wird vom
  `build.rs` von `leo4-rust-bridge` automatisch
  behoben. `*-linux-android*` (C6) auf v1.x
  verschoben, mit demselben Pfad-Scope.

- *Leo4.Platform-Layer*. `lake/Leo4/Leo4/Platform.lean`
  ist die erste leo4-Lean-OS-Abstraktionsschicht.
  Sie zentralisiert die `.so` / `.dylib` / `.dll`-
  Wahl und die POSIX-only `-Wl,-rpath`-Ausgabe, die
  bislang in `Leo4.Build` hartkodiert war.

- *Windows-IPC, Worker-Seite*. Die fehlende Hälfte
  von Phase 9-4c ist da; `open_windows_pipe` von
  `leo4-rust-worker` öffnet die Named Pipe des
  Dispatchers über `CreateFileW`, mit Retry gegen
  das Spawn-Race. Cross-Compile sauber unter
  `x86_64-pc-windows-gnullvm`.

Wer das Lernmaterial vor dem 2026-05-24 vollständig
gelesen hat: keine Architekturentscheidung hat sich
geändert — das Update protokolliert nur, welche RC-
Blocker gelandet sind und welche user-visible Surface
sich erweitert hat.

= Update — 2026-05-29

Nach dem 2026-05-24-RC-Fortschrittsbatch sind bis zur
v1.0-RC-Markierung drei weitere Work Streams gelandet.
Keiner ändert die Architektur; sie ändern, auf welche
Oberflächen sich Nutzer heute verlassen können.

== Function-Arrow Callback ABI (Phase 10-B1.x)

Phase 10-B1 hat den Function-Arrow-Typ in der IDL +
Wire-Schicht am 2026-05-21 gemangelt. Die Runtime-Seite
landete zwischen 2026-05-28..29 in zwei Hälften:

*Outbound-Richtung* (Rust übergibt einen Rust-Closure an
Lean über `leo4::import!`):

- `leo4-abi` liefert jetzt `RustCallbackRegistry` — eine
  main-seitige Per-`Lean`-Registry, die `u64`
  `callback_id`s prägt und den Closure unter dieser ID
  speichert. RAII `RegistrationGuard` erzwingt den
  Per-Call-Lifetime-Vertrag (SPEC §13a).
- `Lean::callback_registry()` macht die per-instance
  `Arc<RustCallbackRegistry>` zugänglich, sodass die
  Makroschicht ohne Thread-Local-Zustand zugreifen kann.
- Das `leo4::import!`-Makro erkennt
  `fn(T₁,…,Tₙ) -> R`-Parametertypen automatisch: der
  emittierte Wrapper registriert den Closure beim
  Eintritt, codiert die `callback_id` (u64 LE) in den
  kanonischen Args-Puffer, ruft den Shim auf und dropt
  dann den Guard. Generic `impl Fn(...) -> R`
  absichtlich nicht unterstützt.
- `OxiLeanInvoker::{attach_outbound_registry,
  invoke_outbound, register_outbound_dispatch_callback}`
  verdrahten den Rückweg: wenn der OxiLean-Evaluator
  eine Lean-Closure-Dereferenz erreicht, entpackt der
  Bridge-Callback `(callback_id, rest) = (u64 LE
  prefix, &args[8..])` und leitet an den registrierten
  Closure weiter.

*Inbound-Richtung* (Rust empfängt einen Lean-Closure in
einen `#[leo4::export]`-Body) wurde in `83cbbcc` über
`LeanCallback<R, Args>` + den `CallbackInvoker`-Trait
verdrahtet. Beide Hälften verwenden die gleiche
`callback_id: u64`-Wire-Form.

== `oxilean_runtime::driver` IO Walker (#76 P0c)

Der Fork-Branch `0.1.3-leo4-ox7` hat ein neues
`oxilean_runtime::driver`-Modul, das ein elaboriertes
`def main : IO α := …` unter einem installierten
`ExternResolver` zu seinen IO-Effekten treibt. Der Walker
erkennt zum 2026-05-29:

- `IO.pure` / `Pure.pure` — nullary Terminal.
- `IO.bind α β m k` (Arität 4) und `Bind.bind m k`
  (Arität 2 nach impliziter Erasure) — walke `m`, dann
  `k`.
- `@[extern]`-attributierte `Const`-Reduktionen —
  `dispatch_extern_const` gegen die übergebene
  `ExternRegistry`.

Alles andere gibt `DriverError::NotYetImplemented` mit
der Debug-Repräsentation des Ausdrucks zurück.
Die Upstream-API wird unter `cool-japan/oxilean#2`
diskutiert; eine Body-PR-Einreichung ist verschoben, bis
es explizites Maintainer-Feedback gibt.

== Distro-Audit-Infrastruktur + Windows-Support-Floor

Ein neues `just linux-distro-audit <distro>`-Rezept
landet unter `ci/linux-distro-audit/`. Distros sind
datengetrieben via `distros.toml`; der Runner nimmt neue
Einträge automatisch auf, ohne hartcodierte Distro-Namen
im Python-Treiber. Initialsatz (aktuell stabil zum
2026-05-29): archlinux, debian-13, ubuntu-26.04,
fedora-44, alpine-3.22.

Der Windows-Support-Floor ist jetzt auf den offiziell
unterstützten UCRT-Bereich verankert: Windows Vista SP2 +
KB2999226 (NT 6.0) oder Windows 7 SP1 + KB3118401
(NT 6.1), bis Windows 11 / Server 2025+. Die KB-
Installation ist die *Deployment-Sorge des
Downstream-Anwendungsentwicklers* — leo4 redistribuiert
oder dokumentiert keinen Endnutzer-UCRT-Installationsfluss.

== Leaf-Crate-Dedup

Zwei Sibling-Leaf-Crates landen, um vorher zweifach
vendored Code zu deduplizieren:

- `sibling/leo4-oxilean-bootstrap/` — OX5-oxi
  Env-Bootstrap + leo4 Boundary-Primitive-Axiome.
- `sibling/leo4-oxilean-translate/` — OX6-Step-13
  PEG → Legacy-Decl-Translator.

Beide Consumer-Vendor-Dateien kollabieren zu
einzeiligen `pub use <leaf>::*;`-Re-Export-Shims.

= Update — 2026-05-31

== OxiLean Driver Walker — Coverage-Abschluss

Der Walker-Eintrag vom 2026-05-29 beschrieb „v0 deckt
`IO.pure`, `IO.bind` und `@[extern]` Const-Dispatch ab;
alles andere gibt `NotYetImplemented` zurück". Diese
Lückenliste ist jetzt geschlossen. Der Walker vom
2026-05-31 erkennt:

- *`IO.pure`-Familie* über die gesamte Monaden-
  Transformer-Kette — `IO.pure` / `Pure.pure` /
  `EIO.pure` / `EStateM.pure` / `ExceptT.pure` /
  `StateT.pure` / `ReaderT.pure` (plus die
  Underscore-gemangelten Schreibweisen, die das
  Unfold des Elaborators wählen kann).
- *`IO.bind`-Familie* mit derselben Transformer-
  Ketten-Abdeckung. Beta-Applikation der Continuation
  `k` gegen das konkrete Ergebnis von `m` (sofern
  statisch bekannt) gehört ebenfalls zu diesem Batch
  — `IO.bind (IO.pure x) k` reduziert sich jetzt zu
  `k x`, statt `k` als opak zu walken.
- *`@[extern]` Const-Dispatch mit kanonischer ABI-
  Argument-Codierung*. Der Walker senkt statisch
  `Nat` / `String`-Literale, `Bool.true` /
  `Bool.false`, `Unit.unit`, Sized-Integer-Typklassen-
  Projektionen (`OfNat.ofNat <type> n` über
  UInt8..128 / USize / Int8..128 / ISize),
  Signed-Integer-Negation (`Neg.neg`), `Char.ofNat`
  Unicode-Codepunkte, benannte Record- / Variant-
  Ctors (`Prod.mk` / `Subtype.mk` / `Option.some` /
  `Sum.inl` etc.), *und* benutzerdefinierte Record-
  / Inductive-Ctors via Env-Lookup
  (`ConstantInfo::Constructor` →
  `Inductive.ctors.len()`-Diskriminanten-Präfix bei
  Multi-Ctor, rekursiv codierte Felder).
- *Stdlib-`IO.println`-Familie* — `IO.println` /
  `IO.eprintln` / `IO.print` / `IO.eprint` feuern
  direkt gegen `print!`/`println!`/`eprint!`/
  `eprintln!` noch vor dem Resolver, sodass
  Embedder keinen Resolver für den üblichen
  stdout-/stderr-Schreibpfad einziehen müssen.
- *Stdlib-`IO.FS.*`-Familie* — `readFile` /
  `writeFile` / `appendFile` / `removeFile` /
  `createDir` / `createDirAll` / `removeDir` /
  `removeDirAll` / `rename` feuern direkt gegen
  `std::fs`. `readFile` legt die Inhalte als
  `Ok(Some(Lit(Str)))` offen, sodass ein
  umschließendes `IO.bind m k` `k` gegen die Bytes
  beta-applikiert. `std::io::Error` wird via
  `ExternCallError::CallbackFailed` in
  `DriverError::ExternFailed` gewickelt.

Der verbleibende `NotYetImplemented`-Arm feuert nur
noch für Shapes, die *konstruktiv außerhalb des
Geltungsbereichs* liegen: Nicht-IO-Monad-Class-Run-
Projektionen (`StateT.run` / `ReaderT.run` /
`ExceptT.run` — gehören in die LCNF- /
Bytecode-Interpreter-Schicht, *unterhalb* des
Walkers), `IO.FS.Handle.*` (host-seitiges
`File`-Lifetime-Modelling, das der Walker nicht
trägt), Compile-Time-Hooks (`dbg_trace` / `panic!` /
`unreachable!` — vom Elaborator behandelt) und
Float-Literal-Senkung (vom Reducer konstantgefaltet).
Embedder fangen bei Bedarf über den Resolver ab.

cool-japan/oxilean#2 (Driver-API-Koordinations-Issue)
bleibt der Upstream-Beitragspfad; die Einreichung
einer Body-PR ist verschoben, bis die API-Form
explizites Maintainer-Feedback erhält. Stand
2026-05-31 ist die Diskussion seit 3+ Tagen still.

== Rust-Transpile-Translate-Coverage-Tail (#72 OX7)

Der OX7-Typklassen-Step vom 2026-05-27 deckte die
Arithmetik `+` / `-` / `*` / `/` / `%` / `^` +
Vergleich `<` / `<=` / `==` / `=` ab. Der
Translate-Coverage-Tail vom 2026-05-31 schließt den
Rest:

- *BinOp-Coverage-Erweiterung* über `>` / `≥` / `≠` /
  `&&` / `||` / `↔` / `∈` / `∉` / `⊆` / `→` (Unicode-
  Pfeil). `arith_op_to_tc_projection` gibt jetzt eine
  Drei-Arm-`BinOpMapping`-Enum zurück: `Direct(tc)`
  für gewöhnliche Projektionen, `Swapped(tc)` für `>`
  / `≥` (Lean-Stdlib drückt diese als geflippte `<` /
  `≤` aus), `Negated(tc)` für `≠` / `∉` (`¬ (Eq.eq a
  b)` / `¬ Membership.mem a b`). Der Unicode-Pfeil
  `→` reiht sich neben ASCII `->` am
  Spezialfall-Pi-Lowering-Head ein.
- *Explizite `By` / `DotFn` / `Raw` Expr-Arme* —
  jede Variante trägt nun eine handlungsfähige
  Diagnose (Term-Mode-/Axiom-Rewrite-Hinweis für
  `By`, `fun x => …`-Rewrite-Hinweis für `DotFn`,
  Parser-Shape-Coverage-Hinweis für `Raw`). Der
  Expr-Match ist nun erschöpfend über `L4Expr` —
  künftige Parser-Varianten erzwingen einen
  Build-Bruch, beabsichtigte Sicherheit.
- *Decl-Coverage* — `DefinitionByArms`
  (gleichungsbasiertes `def NAME : T | pat₀ =>
  body₀ | …`) entzuckert in eine `Definition`,
  deren Body `fun <binders> => match <last_binder>
  with <arms>` ist. `Mutual { decls }` wickelt die
  inneren Übersetzungen in `OxDecl::Mutual { decls:
  [<translated>] }`. Innere Attribut-Wrapper
  bleiben erhalten.

Translate-Test-Anzahl: 36 → 56 über die drei
Sub-Commits. Die verbleibenden
`TranslateError::Unsupported`-Arme (Class-Binder /
Instance-Term-Body / Open Multi-Item / Dsl /
HashCommand / Omit / Include) tragen erklärende
Meldungen, bleiben aber zurückgestellt — sie
brauchen entweder einen 1→N-Translate-API-Wechsel
oder oxilean-seitige Variant-Unterstützung, um
sauber zu schließen.

= Update — 2026-05-31: RC.2~RC.4 typed-enum closure

Das Flagship-Szenario, das diese Charge schließt:
Ein Nutzer schreibt eine Rust-Enum mit Varianten,
die benannte Felder tragen (Struct-Variant-Summentyp),
und verwendet sie direkt in `#[leo4::export]`.
Konkret etwas wie:

```rust
#[derive(Clone, Debug, LeanMarshal)]
pub enum AdsmtVerdict {
    Sat { model: Vec<(String, String)> },
    Unsat { core: Vec<String>, cert: String },
    Abductive { candidates: Vec<AbductiveCandidate> },
    Unknown { reason: String },
}

#[leo4::export]
pub fn solve(v: AdsmtVerdict) -> u64 { … }
```

Vor RC.2 stieß dies hintereinander auf vier
separate Wände:
`leo4-rust-emit::lean_type_of_mangle` decodierte
keine benutzerdefiniert-nominalen Mangle-Präfixe
(`S_<fqn>_s`, `V_<fqn>_v` etc.) und fiel zu einem
`panic!`-Body-Stub-Wrapper durch; `leo4-rust-emit`
hatte keinen Pfad, eine Spiegel-Lean-`inductive`
für `AdsmtVerdict` zu emittieren, sodass der Nutzer
die Lean-Seite von Hand schreiben musste;
`leo4::import!`s `rust_type_to_idl` gab `None` für
benutzerdefinierte Identifier zurück, was
Multi-Instantiation-Importe ohne `#[leo4(args =
"…")]`-Hinweise brach; und `#[leo4::export]` selbst
lehnte benutzerdefinierte Typen in Param-/Return-
Positionen ab.

== Was jetzt steht

- *Patch 1* (`b260ed8`) — `lean_type_of_mangle`
  decodiert alle 5 benutzerdefiniert-nominalen
  Mangle-Präfixe, sodass die Wrapper-Signatur den
  richtigen Lean-Typ erhält, anstatt zu einem
  `panic!`-Stub durchzufallen.
- *Patch 2* (`b260ed8`) — ein neuer
  `linkme::distributed_slice`-Kanal
  `leo4_abi::rust_exports::USER_TYPES` trägt einen
  `UserTypeEntry` pro `#[derive(LeanMarshal)]`-Stelle
  mit `(fqn, kind, fields, ctors)`. Das Derive-
  Makro emittiert den Eintrag automatisch;
  `leo4-rust-emit` liest den Slice über das neue
  FFI-Symbol `leo4_rust_describe_user_types` und
  emittiert echte Lean-`structure`- /
  `inductive`-Spiegel-Decls mit `deriving
  Leo4.LeanMarshal`. Ein neuer
  `rust_type_to_lean_type`-Übersetzer (syn-basierter
  AST-Walk) behandelt verschachtelte Typen:
  `Vec<(String, String)>` → `Array ((String ×
  String))`, `Vec<u8>` → `ByteArray`, `Result<T,
  E>` → `Except E T` usw.
- *Patch 2 Follow-up* (`cfda354`) — entfernt
  `#[cfg(feature = "rust-exports")]` aus dem
  Derive-Emit, damit nachgelagerte User-Crates
  keine `unexpected_cfg`-Lints sehen. `linkme` wird
  zu einer unbedingten Abhängigkeit; das
  Feature `rust-exports` bleibt als No-Op-Alias
  für Rückwärtskompatibilität.
- *Patch 3* (`29a941f`) — `leo4::import!`s
  `rust_type_to_idl_candidates` liefert alle 5
  Kind-Kandidaten für benutzerdefinierte
  Identifier. Das Makro nimmt das kartesische
  Produkt über die Args, dann gewinnt der erste
  Mangling-JSON-Treffer. `leo4::import! { fn
  solve(v: AdsmtVerdict) -> AdsmtVerdict; }` löst
  Variant nun per Kandidaten-Iteration auf, selbst
  wenn der Export mehrere Instantiierungen hat.
- *Patch 4* (`5d786f0`) — ein strenges
  `rust_type_to_idl` senkt benutzerdefinierte
  Identifier zu `Record { fqn, args }`, sodass
  `#[leo4::export]` sie in Fn-Param-/Return-
  Positionen akzeptiert. Pfade mit Lifetime-Args
  (Cow etc.) werden weiterhin abgelehnt.
  Scope-festgezurrt: das Anbringen von
  `#[leo4::export]` an Enum-/Struct-Items selbst
  ist absichtlich weiterhin ein Parse-Fehler — das
  Typ-Wireformat ist die Aufgabe von
  `#[derive(LeanMarshal)]`.

== Ergebnis

Das Flagship-`AdsmtVerdict`-Szenario wird nun mit
*null handgeschriebenem Lean* in Reverse-Richtung
ausgeliefert. Die automatisch generierte
Wrapper-Lean-Datei enthält sowohl die
Spiegel-Induktive als auch den typisierten
`IO`-zurückgebenden Wrapper:

```lean
inductive AdsmtVerdict where
  | sat (model : Array ((String × String))) : AdsmtVerdict
  | unsat (core : Array String) (cert : String) : AdsmtVerdict
  | abductive (candidates : Array AbductiveCandidate) : AdsmtVerdict
  | unknown (reason : String) : AdsmtVerdict
  deriving Leo4.LeanMarshal

def solve (a0 : AdsmtVerdict) : IO (UInt64) := do …
```

Die Workspace-Test-Anzahl wanderte von 254 → 260
(RC.3) → 262 (RC.4); die relevantesten Crates sind
`leo4-macros-backend` 16 → 22 → 24 und
`leo4-rust-emit` 20 → 29.
