#import "../../template/leo4-book.typ": book
#show: book.with(
  title: "leo4 — Anleitung zum Selbstbau",
  subtitle: "Deutsche Ausgabe",
  author: "윤병익 (leo4-Projekt)",
  lang: "de",
)

= Vorwort

Dieses Buch führt Sie Schritt für Schritt durch den Aufbau von
leo4 aus dem Nichts. Jede Etappe folgt der Reihenfolge, die
das ursprüngliche Projekt selbst durchlaufen hat. Am Ende
besitzen Sie:

- Eine Lean-4-Bibliothek, die das Attribut `@[leo4_export]`
  und die Typklasse `LeanMarshal` zur Verfügung stellt.
- Ein Lake-Plugin, das die Exporte findet, einen stabilen
  Schema-Hash berechnet, Symbole manglet, ein C-Shim
  zusammen mit einer Handshake-Datei erzeugt.
- Einen Rust-Workspace, der das Shim über `libloading`
  lädt, Aufrufe in Canonical-ABI-Bytes kodiert, über die
  Mangling-Tabelle dispatcht und das Ergebnis dekodiert ---
  alles versteckt hinter einem Prozedur-Makro, das eine
  saubere Deklaration im Stil von `fn add(a: u64, b: u64) ->
  u64;` anbietet.
- Optionale Erweiterungen: WIT-Tieferlegung,
  Generic-Record-Unterstützung, gegenseitige Rekursion,
  asynchrones `io<T>`, Mathlib-stilige Carrier-Typen.

Dieses Buch dupliziert nicht `SPEC/*.md`. Die Spezifikationen
sind normativ; dieses Buch baut auf sie zu. Wenn Spezifikation
und Buch sich widersprechen, gilt die Spezifikation.

== Voraussetzungen

- Eine Lean-4-Toolchain, die zur in `lean-toolchain`
  fixierten Version passt (aktuell `v4.29.1`).
- Eine Rust-Toolchain ≥ 1.85 (Edition 2024).
- Einen C-Compiler (`clang` oder `gcc`), den das
  Lake-Plugin über `leanc` ansprechen kann.
- `cargo`, `lake`, `just`, `jq`, `wasm-tools` (für das
  optionale WIT-Kapitel).
- Mehrere Stunden konzentrierte Zeit. Die Pipeline ist
  tief; in einer Mittagspause geht das nicht.

== Aufbau

Dieses Buch folgt der Phasenleiter (`ROADMAP.md` im
veröffentlichten Projekt). Jeder Teil bringt eine einzelne
Fähigkeit von Anfang bis Ende an Land. Nach jedem Teil
können Sie eine Demo laufen lassen und etwas in Bewegung
sehen.

#table(
  columns: (auto, 1fr),
  table.header[*Teil*][*Liefert*],
  [I],   [Lean-Laufzeitbibliothek und das Attribut
          `@[leo4_export]`.],
  [II],  [Lake-Plugin-Gerüst; Admit-Set-Algorithmus für
          Generics.],
  [III], [Die IDL: Typen, Mangling, Schema-Hash. Stabiler
          Vertrag zwischen Lean und Rust.],
  [IV],  [Canonical-ABI-Marshalling: Lean-Seite
          (`LeanMarshal`-Typklasse) und Rust-Seite
          (`LeanMarshal`-Trait).],
  [V],   [C-Shim-Emission: Übersetzungseinheit
          `leo4_call_<mangled>` pro Export.],
  [VI],  [Rust-Loader (`leo4-mslean4`) und das
          `leo4::import!`-Proc-Makro.],
  [VII], [WIT-Tieferlegung und `wasm-tools`-Validierung
          (optional, sauber abtrennbar).],
  [VIII],[Gegenseitige Rekursion + `Cyc<i>`.],
  [IX],  [Asynchrones `io<T>`, WASIp3-Schwesterprojekt.],
  [X],   [Mathlib-stilige Carrier-Typen und Brücken.],
)

Sie können am Ende jedes Teils anhalten und haben ein
funktionierendes, nützliches System. Nach Teil V ist das
Hin und Her zwischen Lean und Rust für Skalare und Strings
bereits da. Nach Teil VIII steht die volle Phase-6-Fläche,
die Produktivcode braucht.

= Teil I --- Die Lean-Laufzeitbibliothek

Beginnen Sie damit, die Lean-seitige Fläche herauszuschälen.
Das Plugin und der Rust-Loader bauen auf Attribute, Typklassen
und den Canonical-ABI-Encoder/Decoder. In der Lean-Bibliothek
wohnen diese.

== Projektaufbau

Legen Sie ein Lake-Paket an:

```
lake/
  Leo4/
    Leo4.lean          -- Top-Level-Re-Export
    Leo4/
      Syntax.lean      -- leo4_constraint-Syntaxkategorie
      Export.lean      -- @[leo4_export]-Attribut
      Marshal.lean     -- LeanMarshal-Typklasse + LeanError
      Resource.lean    -- LeanResource-Marker
      Builtins.lean    -- LeanMarshal-Instanzen für Primitive
      Deriving.lean    -- deriving LeanMarshal-Handler
      Build.lean       -- Benutzerseitige Build-Helfer
    lakefile.lean
```

Die `Leo4`-Bibliothek wird von jedem nachgelagerten Paket
`require`d. Halten Sie die Top-Level-Imports minimal.

== `LeanError` definieren

Jede Lean-seitige fehlschlagbare Operation braucht einen Weg,
Fehlercode + Meldung zurückzugeben. Wir nehmen eine flache
Struktur:

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

Die Fehlercodes folgen `SPEC/canonical-abi.md` §13.
Reservieren Sie `0x00000001` für `decodeError`, `0x00000005`
für Handshake-Mismatch, `0x00000007` für
Return-Buffer-Too-Small, `0x00000064` für Unimplemented.
Definieren Sie sie als `def`-Konstanten in derselben Datei.

== Das Attribut `@[leo4_export]`

`@[leo4_export]` ist ein leeres Marker-Attribut. Das Plugin
entdeckt getaggte Deklarationen über die Attribut-Erweiterung.
Lean 4 stellt `registerBuiltinAttribute` dafür bereit.

```lean
import Lean
namespace Leo4

initialize leo4ExportAttr : Lean.TagAttribute ←
  Lean.registerTagAttribute `leo4_export
    "marks a declaration as a leo4 boundary export"

end Leo4
```

Plausibilitätscheck: Definieren Sie einen trivialen Export in
einem anderen Modul und prüfen Sie, dass
`leo4ExportAttr.hasTag (← getEnv) ``YourModule.add` `true`
liefert.

== Die Typklasse `LeanMarshal`

`LeanMarshal` ist die Typklasse, die jeder grenzüberschreitende
Wert implementiert. Die Encode-Seite hängt Bytes an ein
`ByteArray` an; die Decode-Seite liest daraus und gibt
`(Wert, neuer Offset)` zurück, plus einen Fehlerpfad.

```lean
namespace Leo4

class LeanMarshal (T : Type) where
  canonicalEncode : T → ByteArray → ByteArray
  canonicalDecode : ByteArray → Nat → Except LeanError (T × Nat)

end Leo4
```

Das Lean-`ByteArray` ist ein gepacktes `Array UInt8`. Der
`Nat`-Offset gibt unbegrenztes Indizieren für Sicherheit; die
Längenpräfixe des Wireformats halten die effektive Grenze.

== Builtin-Instanzen

Beginnen Sie mit den Skalartypen: `UInt8`, `UInt16`, `UInt32`,
`UInt64`, `Int8` bis `Int64`, `Float`, `Float32`, `Bool`,
`Char`. Encodieren Sie jeweils als Little-Endian-Bytes;
Decodieren liest dasselbe.

Eine repräsentative Implementierung, `UInt32`:

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

Wiederholen Sie für jedes Primitive. Dann bauen Sie zusammengesetzte
Instanzen:

- `String` --- `u32 len + utf-8 bytes`.
- `List T` --- `u32 len + N Elemente`.
- `Option T` --- `u8 disc + payload`.
- `Except E T` --- `u8 disc + payload`.
- `α × β` --- zwei Elemente konkateniert.
- `Nat` (`bignat`) --- `u32 limb count + LE u64 limbs`.
- `Int` (`bigint`) --- `u8 sign + bignat magnitude`.

Jede Instanz hat beide Richtungen. Schreiben Sie die
Decoder-Bound-Checks aggressiv; der Wire-Input ist per
Definition nicht vertrauenswürdig.

== Der Deriving-Handler

`#[derive(LeanMarshal)]` auf der Rust-Seite und
`deriving LeanMarshal` auf der Lean-Seite synthetisieren beide
feldweise Encode/Decode für benutzerdefinierte Typen. Die
Lean-Seite nutzt dafür `registerDerivingHandler`:

```lean
namespace Leo4.Deriving

open Lean Elab Command Meta

private def mkLeanMarshalHandler (declNames : Array Name)
    : CommandElabM Bool := do
  let env ← getEnv
  for declName in declNames do
    let some (.inductInfo indVal) := env.find? declName
      | return false
    -- Verzweigung nach Form: einzelner ctor → Record, alle
    -- nullary multi-ctor → Enum, gemischt → Variante.
    -- (Detail in Deriving.lean des veröffentlichten Projekts.)
    pure ()
  return true

initialize
  registerDerivingHandler ``Leo4.LeanMarshal mkLeanMarshalHandler

end Leo4.Deriving
```

Der Körper des Handlers ist der harte Teil: Er läuft die
Konstruktoren der induktiven Definition ab, baut die
Encode-Arme (ein Match-Arm pro ctor, der den Diskriminator
pushed und dann jedes Feld kodiert), dann die Decode-Arme
(einer pro Diskriminator, der die inneren Felder herauszieht).
Für die Phase-6-Mutual-Unterstützung kommen alle Mitglieder
einer `mutual ... end`-Gruppe in einen `mutual ... end`-Block
mit `partial def`-Encodern/-Decodern + einer Instanz pro
Mitglied.

Bringen Sie zuerst eine Single-Shape-Implementierung
(z.~B.~nur Records) an Land. Enum-, Varianten- und
Mutual-Unterstützung können in späteren Phasen folgen.

== Plausibilitätscheck

Schreiben Sie eine Lean-Datei außerhalb der Leo4-Bibliothek:

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

Sie sollten 16 kodierte Bytes (zwei `f64`s) sehen, die zurück
hin und her gehen. Wenn nicht, reparieren Sie den
Encoder/Decoder, bevor Sie weiterziehen.

= Teil II --- Das Lake-Plugin-Gerüst

Das Lake-Plugin ist eine `lean_exe`, die nach `lake build`
läuft. Sie läuft jede `@[leo4_export]`-Definition im
Benutzerpaket ab, berechnet ihr Admit-Set (für Generics) und
emittiert die in Kapitel 3 des Lernmaterials aufgelisteten
Artefakte.

== Projektaufbau

```
lake/
  Leo4Plugin/
    Leo4Plugin.lean      -- top-level
    Leo4Plugin/
      AdmitSet.lean      -- IDLType + UserDecl ADT, admit-set-Algo
      Mangling.lean      -- mangleType, Schema-Hash
      Emit.lean          -- Dateischreiber, JSON-Formen
      Main.lean          -- der runPlugin-Treiber
    Main.lean            -- exe-Entry-Point
    lakefile.lean
```

`lakefile.lean` deklariert das Paket, `require`d `Leo4` (die
Laufzeitbibliothek aus Teil I) und stellt
`lean_exe leo4plugin` bereit, dessen Root-Modul `Main.lean`
ist.

== Exporte entdecken

Das Plugin läuft als alleinstehende ausführbare Datei. Es
erhält den Modulnamen des Benutzers als
Kommandozeilenargument:

```
$ lake exe leo4plugin Sample
```

Der Entry lädt die kompilierten Module des Benutzers über
`Lean.importModules (loadExts := true)` und durchsucht dann
die Umgebung nach `@[leo4_export]`-getaggten Decls:

```lean
def gatherExports (env : Environment) : Array Name := Id.run do
  let mut out : Array Name := #[]
  for (n, _) in env.constants do
    if Leo4.leo4ExportAttr.hasTag env n then
      out := out.push n
  return out
```

Das ergibt eine sortierte Liste von `Name`-Werten zur
Analyse. Für jeden wird der Typ der Funktion abgerufen, in
Generic-Parameter / Wert-Parameter / Rückgabetyp aufgeteilt
und jeder auf die IDL heruntergelegt.

== Die IDLType-ADT

Die IDL-Repräsentation des Plugins ist eine
Lean-Induktive, die das Rust-seitige `schema-idl::IDLType`
spiegelt. Definieren Sie sie in `AdmitSet.lean`:

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

Konstruktor für Konstruktor ist das die kanonische IDL. Die
Rust-Seite spiegelt sie exakt (abgesehen von
Rust-Benennungskonventionen); die Mangling-Regel (Kapitel 5)
mappt jeden auf einen stabilen ASCII-String.

Die `UserDecl`-ADT sammelt Nominal-Typ-Deklarationen:

```lean
inductive UserDecl where
  | record   (fqn) (generics : Array Name) (fields : Array (Name × IDLType))
  | enumT    (fqn) (cases : Array Name)
  | variant  (fqn) (generics : Array Name) (cases : Array (Name × Array IDLType))
  | resource (fqn) (generics : Array Name)
  | mutual   (members : Array UserDecl)
  | externalMarshal (fqn) (generics : Array Name)
```

Die zwei zusätzlichen Konstruktoren (`mutual` und
`externalMarshal`) kommen in Phase 6 und Phase 8 dazu; sie
stehen hier, damit die ADT von Anfang an vollständig ist.

== Einen Export ablaufen

Für jedes getaggte `Name` holen Sie sich seinen
`ConstantInfo`, teleskopieren seinen Typ und legen jeden
Binder herunter:

```lean
def analyzeExport (n : Name) : MetaM (Option ExportAnalysis) := do
  let env ← getEnv
  let some info := env.find? n | return none
  Meta.forallTelescope info.type fun args body => do
    -- Binder klassifizieren in:
    --   - implizit kind-typed → Generic-Typ-Parameter
    --   - implizit value-typed → gelöschter Wert-Parameter
    --   - inst-implicit → Typklassen-Constraint
    --   - explizit → Wert-Parameter an der Grenze
    -- Dann jeden Wert-Parameter-Typ über exprToIDLSubst herunterlegen.
    sorry
```

`exprToIDLSubst` ist der rekursive Typabsenker: Gegeben eine
Lean-`Expr` und eine Substitution (von Generic-Bindern zu
konkreten `IDLType`-Werten) liefert er den entsprechenden
`IDLType` oder `none`, falls keine Absenkung möglich ist.
Sonderfälle für `List`, `Option`, `Except`, `Prod`, `IO` und
die Self-Abkürzung. Benutzerdefinierte induktive Typen
landen je nach Form auf
`record`/`variant`/`enumT`/`resource`.

Das entscheidende Detail: Nutzen Sie
`Meta.forallTelescope` (ohne Reducing), damit die ursprüngliche
`IO α`-Form überlebt. Die reduzierende Variante entfaltet
`IO α = IO.RealWorld → EStateM …` und bringt damit
spurious `IO.RealWorld`-Parameter hervor.

== Der Admit-Set-Algorithmus

Für einen Export mit Generic-Parametern zählt das Plugin alle
Instanziierungen auf, die die Binder-Constraints erfüllen. Für
jede Kombination produziert es eine eigene IDL-Signatur und
einen eigenen mangled Namen. Der Algorithmus:

1. Für jeden Generic `T_i` das Admit-Set bestimmen: die Menge
   der `IDLType`-Werte, die er annehmen kann. Standard: alle
   Primitive (`unboundedAdmitSet`). Mit Klassen-Constraints:
   Schnitt mit dem `classAdmitSet` jeder Klasse.
2. Kartesisches Produkt berechnen. Jedes Tupel ist eine
   Instanziierung.
3. Für jede Instanziierung in die Parameter-Typen einsetzen
   und das `paramInfo`-Array produzieren.

Phantom-Generics (Binder, die nirgends referenziert werden)
überspringen die kombinatorische Explosion --- es wird eine
Instanziierung emittiert, in der die Phantom-Slots `none`
sind.

Dieser Algorithmus steht im `analyzeExport` von `Main.lean`
in der veröffentlichten Codebase. Lesen Sie ihn einmal,
bevor Sie ihn neu implementieren; die Edge Cases (höher
geartete Generics, Wert-Generics, Generic-Argumente in
Self-rekursiven Typen) brauchen eine Weile.

= Teil III --- Mangling und Schema-Hash

Sobald Sie `IDLType`-Werte + eine Liste der Exporte + die
entdeckten Benutzertypen haben, können Sie die stabile
Textform produzieren, die der Schema-Hash konsumiert.

== `mangleType`

`mangleType : IDLType → String` ist zwischen Lean und Rust
byte-identisch. Jeder Konstruktor mappt auf ein festes Token:

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

Die volle Regel steht in `SPEC/mangling.md` §2.

== Der volle mangled Name

```
leo4__<pkg_seg>__<iface>__<fname>__<arg_mangles>__h<schema_hash>
```

`arg_mangles` verbindet die mangled Formen jedes
Parameter-Typs mit Unterstrichen. Der Schema-Hash ist die
FNV-1a-64 der *normalisierten IDL-Form* (Text), gerendert als
13-Zeichen-base32lc (Kleinbuchstaben, ohne Padding).

FNV-1a-64 ist geradlinig: Offset-Basis
`0xCBF29CE484222325`, Prime `0x00000100000001B3`, jedes Byte
XORen, dann multiplizieren. Base32lc verwendet das Alphabet
`abcdefghijklmnopqrstuvwxyz234567` (RFC 4648 Kleinbuchstaben,
ohne Padding).

== Kanonische IDL rendern

`renderCanonical : Config → Array UserDecl → Array Member → Bool → String`
erzeugt den Text:

```
package leo4-sample;
interface Sample {
  record Sample.Point { x: f64, y: f64 };
  variant Sample.Tree { leaf, node(Self, Self) };
  func add(_0: u64, _1: u64) -> u64;
  func midpoint(_0: Sample.Point, _1: Sample.Point) -> Sample.Point;
}
```

Zwei Modi: `pretty := true` (Zeilenumbrüche, Einrückung) für
die `.leo4-schema`-Datei auf Platte; `pretty := false`
(kollabiert, einzelnes Leerzeichen zwischen Tokens) für den
Schema-Hash-Input.

User-Decls innerhalb ihrer Bande nach FQN sortieren (Records
und Enums in Bande 0, Resources in Bande 1, Mutual-Cluster in
Bande 0 mit Beibehaltung der Quellreihenfolge). Funktionen nach
Name sortieren. Determinismus ist nicht verhandelbar --- der
Hash hängt von byte-identischer Ausgabe ab.

Der Hash-Input ist die *kollabierte* Form. FNV-1a über seine
UTF-8-Bytes laufen lassen, um ein `UInt64` zu erhalten;
Big-Endian in einen base32-String konvertieren, der das
Suffix bildet.

== Plausibilitätscheck

Schreiben Sie ein kleines Fixture-Programm (Lean-Seite):

```lean
@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b
```

Lassen Sie `lake exe leo4plugin Sample` laufen und prüfen Sie:

- `tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-schema`
  ist eine vernünftige Textdatei.
- `tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-handshake`
  enthält im JSON-Feld `schema_hash` einen 16-Zeichen-
  base32lc-Hash.
- Der Hash ist über mehrere Läufe hinweg stabil.

Implementieren Sie dann parallel den Rust-Spiegel
(`crates/schema-idl/`) und fügen Sie ein
Cross-Impl-Harness (`tests/mangling/`) hinzu, das die
Lean-Ausgabe mit der Ausgabe von `leo4c mangle <schema>`
vergleicht. Beide müssen byte-für-byte übereinstimmen.

= Teil IV --- Canonical-ABI-Marshalling

Die Lean-Bibliothek hat ihre `LeanMarshal`-Typklasse. Die
Rust-Seite braucht einen passenden Trait. Beide müssen
identische Bytes für jeden gemeinsam genutzten Wert-Typ
produzieren.

== Rust-Trait

```rust
pub trait LeanMarshal: Sized + 'static {
    fn canonical_encode(&self, buf: &mut Vec<u8>);
    fn canonical_decode(buf: &[u8], off: usize)
        -> Result<(Self, usize), LeanError>;
}
```

`Vec<u8>` zum Encoden (wächst bei Bedarf), `&[u8] + off` zum
Decoden (gleiche Form wie auf der Lean-Seite). `LeanError`
trägt einen `u32`-Code + `String`-Detail, passend zum
Lean-`Leo4.LeanError`.

== Primitive-Impls

Für jedes Rust-Primitiv eine direkte Impl schreiben:

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

Für jedes Primitiv wiederholen. Das exakte LE-Verhalten
zählt --- schreiben Sie die Bytes, die Leans
`(n.toUInt8, ..., (n >>> 24).toUInt8)`-Kette produziert.

== Das Conformance-Harness

Die beiden Seiten müssen byte-für-byte übereinstimmen. Bauen
Sie ein Fixture:

```
tests/conformance/
  fixtures/
    u32.lean       -- emittiert `u32 42` als Bytes via Leo4.LeanMarshal
    u32.rs         -- emittiert `42u32` als Bytes via leo4-abi
    point.lean     -- Record-Beispiel
    point.rs       -- dasselbe
    ...
  run.sh
```

`run.sh` läuft beide Fixtures mit demselben logischen Wert,
vergleicht ihre Byte-Ausgaben und schlägt fehl, falls ein
Paar divergiert. Das ist der Test, der subtile
Byte-Reihenfolge-Fehler vor dem Ausliefern fängt.

Mindestens ein Fixture pro Typ: jedes Primitive, jede
zusammengesetzte Form (List, Option, Result, Tuple) und
mindestens zwei benutzerdefinierte Typen (Record, Variante).

= Teil V --- C-Shim-Emission

Das C-Shim ist die Stelle, an der Leans natives ABI
(`lean_object*`, `lean_alloc_ctor`, `lean_io_result_*`, …)
auf den Byte-Strom des Canonical ABI trifft. Das Plugin
generiert pro Paket eine `.c`-Datei mit einem
`LEO4_EXPORT int32_t leo4_call_<mangled>(...)`-Entry pro
Export × Instanziierung.

== Aufbau der Shim-Quelldatei

Die Übersetzungseinheit des Shims hat:

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

/* extern-Deklarationen pro Helfer folgen */
extern uint64_t leo4_lean__leo4__sample__Sample__add__u64_u64__h<hash>(uint64_t, uint64_t);

LEO4_EXPORT int32_t leo4_call_leo4__sample__Sample__add__u64_u64__h<hash>(
    leo4_arena_t* arena,
    const uint8_t* args_ptr, size_t args_len,
    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)
{
    /* Argumente decoden */
    /* aufrufen */
    /* Rückgabewert encoden */
}
```

Die Signatur steht fest (`SPEC/canonical-abi.md` §14). Der
Loader bindet via `dlsym` daran; das Makro generiert
Rust-Code, der dort hineinruft.

== Typweise Handler

Die Kerndaten\-struktur des Shim-Emitters ist `TyHandler`:

```lean
private structure TyHandler where
  cType        : String   -- z.~B.~"uint64_t"
  externCType  : String   -- C-Typ in der extern-Deklaration
  ownsRef      : Bool     -- lean_dec am Ende nötig?
  scalarKind   : Option String  -- "uint8" usw. für ctor-Accessoren
  ctorScalarSz : Nat
  decodeBlock  : String → String → String  -- (var, cleanup) → C
  encodeBlock  : String → String → String
  boxExpr      : String → String  -- value → lean_object*
  unboxExpr    : String → String  -- lean_object* → value
```

Für jeden IDL-Typ löst der Emitter einen `TyHandler` auf.
Skalare nutzen einen generischen `scalarHandler`. Strings
nutzen `stringHandler` (delegiert an einen Laufzeit-Helfer).
List / Option / Result / Tuple sind höherwertig ---
`listHandler ih` nimmt den Handler des inneren Typs entgegen
und hüllt ihn ein.

Benutzerdefinierte Records produzieren einen `recordHandler`
über die Feldhandler. Varianten haben ihren eigenen Emitter,
der pro (fqn, args)-Instanziierung zwei Helfer-Funktionen
erzeugt (z.~B.~`leo4_dec_Sample_Tree` und
`leo4_enc_Sample_Tree`), die jeweils Disc + Payload erledigen.

Self-Referenzen in Varianten rufen denselben Helfer rekursiv
auf. Mutual-Cluster verwenden `Cyc<i>`-Referenzen, die zur
Emissionszeit auf den Helfer des Peers aufgelöst werden
(Kapitel VIII).

== Die Hauptschleife des Renderns

```lean
def renderOneShim (cfg userDecls a schemaHash params ret) : String :=
  let mangled := mangle cfg.pkg cfg.iface a.fname (params.map ...) schemaHash
  let entry  := s!"leo4_call_{mangled}"
  let helper := s!"leo4_lean__{mangled}"
  -- paramHs : Array TyHandler aus params via handlerFor bauen.
  -- retH    : TyHandler aus ret bauen.
  -- Ist einer der Handler `none`, einen LEO4_ERR_UNIMPLEMENTED-Stub emittieren.
  -- Andernfalls den vollen decode → invoke → encode-Körper emittieren.
  ...
```

Jeder Export ergibt etwa 30--100 Zeilen generierten C-Code.
Das Ergebnis kompiliert via `leanc` (im Grunde clang mit Leans
Include-/Library-Pfaden vorkonfiguriert) zu einem `.so`.

== Plausibilitätscheck

Nach dem Lauf von `lake exe leo4plugin Sample` inspizieren
Sie `<pkg>.leo4-shim.c`. Ein skalares
`add(u64, u64) -> u64` sollte so aussehen:

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

Lassen Sie `leanc` (oder `cc` mit den passenden Flags) auf
dieses `.c` plus das `.c` des Lean-Wrapper-Moduls laufen, um
`<pkg>.leo4-shim.so` zu erzeugen. Linken Sie die `.so` des
Benutzer-Pakets (aus `precompileModules` von `lake build`)
über RPATH, damit der Wrapper die kompilierten Exporte des
Benutzers aufrufen kann.

= Teil VI --- Rust-Loader und das `import!`-Makro

Sie haben ein Shim-`.so`, eine Handshake-Datei und eine
Mangling-Tabelle. Jetzt bindet die Rust-Seite daran.

== `leo4-mslean4` --- der Loader

`crates/leo4-mslean4/` exponiert `Lean::open`:

```rust
pub struct Lean { /* libloading::Library + Meta */ }

impl Lean {
    pub fn open(
        so_path: impl AsRef<Path>,
        handshake_path: impl AsRef<Path>,
    ) -> Result<Self, LeanError> {
        // 1. Handshake-JSON lesen; schema_hash + wrapper_init_symbol extrahieren.
        // 2. schema_hash gegen die Rust-seitige Konstante verifizieren.
        // 3. `Library::new(so_path)` via libloading.
        // 4. Lean-Laufzeit einmal pro Prozess initialisieren
        //    (`lean_initialize_runtime_module`, dann das
        //    `initialize_<X>`-Symbol des Wrappers).
        // 5. Funktionszeiger pro mangled Symbol cachen.
        ...
    }

    pub fn call_shim(
        &self,
        mangled_body: &str,
        args: &[u8],
        ret: &mut [u8],
    ) -> Result<usize, LeanError> {
        // `leo4_call_<mangled>` via dlsym auflösen (gecached).
        // Mit (arena=NULL, args_ptr, args_len, ret_ptr, ret_cap,
        // &ret_len) aufrufen. Den int32_t-Status in ein Result
        // umwandeln.
        ...
    }
}
```

Die Laufzeit-Initialisierung ist einmal pro Prozess via
`std::sync::Once`. Das `initialize_<X>`-Symbol des
Wrapper-Moduls liefert Erfolg im Stil von
`lean_io_result_is_ok`; vor dem Dispatchen eines
Benutzeraufrufs prüfen.

== `leo4-macros-backend` --- der Makro-Expander

`leo4::import!` ist ein Funktions-Prozedur-Makro
(`#[proc_macro]`). Es parst eine Eingabe, die einem
extern-Block ähnelt, schlägt jedes `fn` in der zur Build-Zeit
verfügbaren Mangling-JSON nach und emittiert einen
Rust-Wrapper:

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

Die Aufgabe des Makros ist es, das richtige `MANGLED_BODY`
zu wählen. Es liest `LEO4_MANGLING_FILE` (von `leo4-build`
gesetzt) und matched den Funktionsnamen + die IDL-Form jedes
Arguments (berechnet via `rust_type_to_idl`). Für Generic-
Exporte mit mehrfacher Instanziierung lässt ein
`#[leo4(args = "u64,str")]`-Attribut den Nutzer explizit
wählen.

== `leo4-build` --- der Build-Skript-Helfer

```rust
pub fn wire(lake_build_dir: &str) -> Result<(), String> {
    // Absolutpfad von Shim-.so und Handshake-Datei auflösen.
    // `cargo:rustc-env=LEO4_SHIM_SO=…`
    //       `cargo:rustc-env=LEO4_HANDSHAKE_FILE=…`
    //       `cargo:rustc-env=LEO4_MANGLING_FILE=…`
    //       `cargo:rerun-if-changed=…` emittieren.
    ...
}
```

Das ist es, was `env!("LEO4_SHIM_SO")` in der `main.rs` des
Benutzers funktionieren lässt. Das Makro liest
`LEO4_MANGLING_FILE` (auch via `env!`), um die Mangling-Tabelle
aufzulösen.

== Alles zusammenbauen

Eine vollständige Consumer-Crate hat:

```
my-app/
  Cargo.toml         # [dependencies] leo4 = "..."; [build-dependencies] leo4-build = "..."
  build.rs           # leo4_build::wire(<path>)
  src/main.rs        # mod sample { leo4::import! { ... } } fn main() { ... }
```

`cargo run` aus `my-app/` baut die Makro-Expansionen des
Wrappers, linkt das Shim-`.so` und der Laufzeit-Aufruf
funktioniert von Anfang bis Ende.

= Teil VII --- WIT-Tieferlegung (optional)

Die IDL ist eine Obermenge von WIT; Sie können jede leo4-IDL
in eine WIT-Datei tieferlegen, die Component-Model-Tools
konsumieren können.

== `leo4c lower`

Eine kleine Rust-CLI (`crates/leo4c`), die eine
`.leo4-schema` liest und eine `.wit`-Datei emittiert. Die
Konvertierung:

- IDL `record R { f: u32 }` → WIT `record r { f: u32 }`.
- IDL `variant V { a, b(string) }` → WIT
  `variant v { a, b(string) }`.
- IDL `resource X` → WIT `resource x`.
- IDL `enum E { a, b }` → WIT `enum e { a, b }`.
- IDL `flags F { x, y }` → WIT `flags f { x, y }`.
- IDL `func f(_0: T) -> R;` → WIT
  `f: func(_0: t) -> r`.

Selbst-rekursive Varianten werden in WIT über einen
`resource`-Typ ausgedrückt (WIT erlaubt keine direkte
Selbst-Rekursion in Varianten-Payloads). Die Tieferlegung
erkennt die Rekursion und substituiert entsprechend.

Validieren Sie die Ausgabe via:

```
$ wasm-tools component wit <pkg>.wit  # parse + pretty-print
$ wit-bindgen markdown <pkg>.wit       # API-Doku generieren
```

Beide müssen die Ausgabe ohne Fehler akzeptieren.

= Teil VIII --- Gegenseitige Rekursion + `Cyc<i>`

Phase 6 des Originalprojekts. Bis hierher lief Rekursion über
`Self` (eine Deklaration, die sich selbst rekursiv referenziert).
Gegenseitige Rekursion braucht einen Weg, mit dem zwei
Deklarationen einander namentlich nennen.

== Erweiterungen der IDL-Grammatik

```
mutual_decl = "mutual" "{" nominal_decl nominal_decl { nominal_decl } "}" ";"
cyc_type    = "Cyc" "<" unsigned_int ">"
```

Ein `mutual`-Block enthält ≥ 2 nominale Deklarationen, die
sich einen `Cyc<i>`-Namensraum teilen. Innerhalb jedes
Mitglieds bezieht sich `Cyc<i>` auf das `i`-te Mitglied der
Gruppe in Quellreihenfolge.

== Mangling-Regel

`Cyc<i>` → `c<i>c`, wobei `<i>` der ASCII-Dezimalindex ist.
Der Schema-Hash wird über den vollständigen normalisierten
Text inklusive `Cyc<i>`-Tokens berechnet, sodass das
Umsortieren der Mitglieder den Hash rotiert.

== Plugin-Arbeit

Das Lean-Plugin erkennt einen Mutual-Cluster über das
`InductiveVal.all`-Array. Wenn `iv.all.length > 1`, wird zur
Funktion `walkMutualGroup` verzweigt, die:

1. Für jedes Mitglied `walkUserDecl` mit `mutualMembers =
   iv.all` aufruft, sodass Peer-Referenzen zu `Cyc<i>` umgeschrieben
   werden.
2. Das resultierende `UserDecl`-Array in `UserDecl.mutual`
   einpackt.

Der Varianten-Helfer-Handler des Shim-Emitters greift
`Cyc<i>`-Payloads auf und emittiert Cross-Calls zu den Peers
`leo4_dec_<seg>` / `leo4_enc_<seg>`. Beide Helfer leben in
derselben Übersetzungseinheit; eine Forward-Deklaration ganz
oben im Shim-Header macht sie an der Aufrufstelle sichtbar.

Der Deriving-Handler emittiert pro Cluster einen
`mutual partial def … end`-Block und dann eine
`instance : LeanMarshal X` pro Mitglied. Cross-Decl-
Payload-Referenzen routen direkt durch
`<peer>._leo4_encode` / `_decode` des Peers statt durch
Typklassen-Dispatch (das würde die noch unfertige Instanz
nach vorne referenzieren).

== Rust-Derive

Rust akzeptiert Forward-Referenzen zwischen Top-Level-
`impl`-Blöcken im selben Modul frei. Die Pass-Through-
`LeanMarshal`-Impl für `Box<T>` in
`leo4-abi/composites.rs` lässt rekursive Rust-Enum-Typen wie
`Expr { Lit(u64), Seq(Box<Stmt>) }` sized sein, ohne weitere
Makroarbeit. `#[derive(LeanMarshal)]` behandelt jedes Enum
unabhängig und der Zyklus löst sich zur Compile-Zeit auf.

== Plausibilitätscheck

Bringen Sie ein Beispiel mit Mutual-Cluster an Land:

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

Nach `lake exe leo4plugin Sample` sollte das Schema
folgendes enthalten:

```
mutual { variant Sample.Expr { lit(u64), seq(Cyc<1>) }; variant Sample.Stmt { nop, block(Cyc<0>) }; };
```

Die Rust-Seite definiert die spiegelnden Enums + per Hand
geschriebene (oder derivierte) `LeanMarshal`-Impls und ruft
`exprIsLit` durch das Makro auf.

= Teil IX --- Asynchrones io<T> + WASIp3

Phase 7. Das nutzerseitige API bleibt auf beiden Zielen sync
(per Designentscheidung vom 2026-05-20); WASIp3 erlaubt einem
sync-WASM-Export, intern asynchrone WASIp3-Futures zu
`block_on`en.

== IDL-Oberfläche

Leans `def f : IO α` legt sich in `exprToIDLSubst` des
Plugins zu `IDLType.io α` herunter. Die kanonische IDL
rendert das als `future<α>` (Phase-7-Lift). Der Rust-
schema-idl-Parser desugart `future<α>` zur Parse-Zeit zu
`FuncDecl { effect: Async, ret: α }`, damit das Hin und Her
symmetrisch bleibt.

== Shim-IO-Entpacken

Der Lean-Wrapper für `IO α`-Exporte gibt auf C-Ebene ein
`lean_io_result α` zurück. Das Shim umhüllt den Aufruf:

```c
lean_object* io_res = leo4_lean__<mangled>(args);
if (!lean_io_result_is_ok(io_res)) {
    lean_dec(io_res); *ret_len = 0;
    return LEO4_ERR_IO_FAILED;
}
RetType r = scalarUnbox(lean_io_result_get_value(io_res));
lean_dec(io_res);
// r encoden...
```

`scalarUnbox` dispatcht pro cType: `lean_unbox_uint64` /
`lean_unbox_uint32` / `lean_unbox` / `lean_unbox_float` /
`lean_unbox_float32`. Signed und Unsigned teilen sich
dieselbe C-Breite; die Konvertierung an der Aufrufstelle
erhält die Vorzeicheninterpretation.

== WASIp3-Schwester

Ein eigenständiges Cargo-Projekt unter
`sibling/leo4-wasip3/`, *nicht* Mitglied des Hauptworkspaces.
Pinnt stable Rust + das Ziel `wasm32-wasip2`; hängt vom
`wasip3`-Crate ab (das WASIp3-API-Bindings als Kompatibilitäts-
Shims auf wasip2s Component-Model ausliefert).

Die Schwester implementiert `leo4_wasip3::Lean::open` analog
zu `leo4_mslean4::Lean::open`, aber der Dispatch läuft über
WASIp3-Host-Imports (definiert in einer WIT-Datei, die der
Host implementiert). `futures::executor::block_on` treibt
jede async Import, während das nutzerseitige Rust-API sync
bleibt.

== Plausibilitätscheck

Bringen Sie einen `IO`-geprägten Sample-Export an Land:

```lean
@[leo4_export]
def asyncDouble (n : UInt64) : IO UInt64 := return n * 2
```

Das Schema sollte
`func asyncDouble(_0: u64) -> future<u64>` zeigen. Der
Rust-Aufrufer schreibt `fn asyncDouble(n: u64) -> u64;` und
erhält `asyncDouble(21) == 42`.

= Teil X --- Mathlib-kompatible Carrier-Typen

Phase 8. leo4 bleibt nach ROADMAP §8 Mathlib-unabhängig --- die
Laufzeitbibliothek importiert Mathlib nicht. Aber sie liefert
Carrier-Typen aus (`LeanRat`, `LeanU128/I128`,
`LeanComplexF*x2`, `LeanF16/BF16/F128` nightly), die hin- und
hergehen mit den abstrakten Mathlib-Typen (`ℚ`,
`ZMod (2^128)`, `Complex ℝ`, `ℝ`).

== Wide-Ints

`Leo4.LeanU128 { lo : UInt64, hi : UInt64 }` und passendes
`LeanI128`. Der Wire ist 16 Bytes LE; das feldweise Encoden
aus `deriving LeanMarshal` produziert denselben Byte-Strom
wie Rusts `u128::to_le_bytes()`. Das Rust-Makro mappt das
nackte `u128` über `rust_type_to_idl` auf die
`Leo4.LeanU128`-IDL-Form.

== Maschinen-Komplex

`Leo4.LeanComplexF{32,64}x2 { re, im : Float* }`. Die
Benennungskonvention `F<bits>x<components>` erweitert sich
später auf Quaternion- (`xN=4`) / Oktonion-Carrier (`xN=8`).

== Nightly-Floats

`LeanF16`, `LeanBF16`, `LeanF128` plus die passenden
Komplex-Carrier, hinter dem Cargo-Feature `nightly-floats`
gegated. Rusts `f16` / `f128`-Primitiven sind nightly via
`#![cfg_attr(feature = "nightly-floats", feature(f16, f128))]`;
`bf16` hat noch keine native Rust-Primitive, also tragen wir
das Bitmuster als `u16`-Newtype.

Die Lean-Seite hat keine nativen `Float16` / `Float128`; die
Carrier umhüllen rohe Bitmuster (`UInt16` oder zwei
`UInt64`s).

== External-Marshal (`Rat`)

Leans Kern-`Rat` hat beweis-tragende Felder (`den_nz`,
`reduced`), die das Plugin nicht herunterlegen kann. Der
`UserDecl.externalMarshal`-Pfad behandelt sie auf IDL-Ebene
als opake Blobs; der Shim-Emitter routet Encode / Decode
durch Lean-emittierte C-rufbare Helfer
(`leo4_marshal_Rat_dec` / `leo4_marshal_Rat_enc`), die
`Leo4.LeanMarshal.canonicalDecode/Encode` umhüllen. Das Shim
erledigt den `uint8_t* ⇄ ByteArray`-Klebstoff via
`lean_alloc_sarray` + `leo4_memcpy`.

== Mathlib-Brücken

Jeder Carrier wird mit einem opt-in
`Leo4.MathlibBridge.<Sub>`-Modul ausgeliefert. Die Brücken:

- `Wide` --- `LeanU128/I128 ↔ Nat / Int / BitVec 128 / ZMod (2^128)`.
- `Complex` --- `LeanComplexF{32,64}x2 → ℂ` via
  `Float.toReal`. Die Rückrichtung
  `ℂ → LeanComplexF*x2` ist `noncomputable` (Mathlibs ℝ hat
  kein konstruktives `→ Float`).
- `NightlyFloats` --- IEEE-754-Bit-Decode `LeanF{16,BF16,128}
  → ℝ` via direkter Arithmetik auf `Nat`-Feld-Extrakten. Die
  Rückrichtung läuft durch `Rat` (computable Teilmenge von ℝ)
  unter IEEE-korrekter Round-to-Nearest-Even.
- `Rat` --- Lean-Kern-`Rat` → `ℝ` / `ℂ` totale Einbettungen
  via Mathlibs `Rat.cast`.

Rundungsmodus-Policy: IEEE-754 Round-to-Nearest-Even (RTNE).
Das ist es, was `Float.div` und die Host-FPU implementieren,
sodass der abstrakt-Real-Rückpfad konsistent mit dem
Hin-und-Zurück bleibt, das nativer Code bereits ausführt.

== Schluss

Sie haben jetzt eine Ende-zu-Ende-leo4-Implementierung. Die
nächsten Schritte sind Stretch Goals: WIT-Lowering-Verfeinerungen,
zusätzliche Mathlib-Brücken, das native Ziel
`wasm32-wasip3`, wenn es sich stabilisiert, und der
schema-idl-`ConstraintExpr<Atom>`-typed AST, wenn ein
Konsument ihn braucht.

Die vollständige Referenzimplementierung steht bei
`github.com/Honey-Be/leo4`. Vergleichen Sie Ihre Version
gegen sie, während Sie voranschreiten; die Commit-Messages
dort benennen jeden Schritt und erklären, warum das Design
dort gelandet ist, wo es ist.

Happy hacking.

= Update — 2026-05-24

Wenn Ihr Referenz-Checkout nach dem 2026-05-24 datiert,
hat sich die Implementierungsreihenfolge an einigen
Stellen verschoben. Keine dieser Änderungen verändert
die Kernarchitektur; sie schließen alle v1.0-RC-Lücken,
die der ursprüngliche Phasen-Ladder als TODO führte.

== OX6: PEG-basierter Lean-4-Parser als Sibling-Crate

Die OX3 / OX4 textuelle Pre-Rewrite-Kette in
`leo4-oxilean-build` (`lean4_normalize` & Co.) war ein
Notbehelf und stieß an ihre Grenzen — Operator-Precedence,
String-Interpolation, Ctor-Namensauflösung verlangten
echte Grammatikarbeit. Eine Sibling-Crate unter
`sibling/leo4-lean4-parse/` mit der `peg`-Crate
aufbauen. Mit `def NAME … := VALUE` beginnen, die
Ausdrucks-Precedence über PEGs `precedence!` ergänzen,
dann Lean-4-Surface-Formen einzeln (~25 Sub-Steps
insgesamt) aufschichten. AST-Formen spiegeln
`oxilean-parse` v0.1.2 — Downstream-Code, der
`oxilean_parse::Decl` schon konsumiert, bekommt einen
Translator (`leo4_translate`-Modul in
`leo4-oxilean-build`) statt eines Rewrites.

Mit einem Integrationstest
(`tests/oxilean_cross_check.rs`) verifizieren, der
beide Parser über ein gemeinsames Corpus laufen lässt
— jede Eingabe, die `oxilean-parse` akzeptiert, muss
auch `leo4-lean4-parse` mit übereinstimmender
Decl-Anzahl + Name + Kind-Tag akzeptieren.

`leo4-oxilean-build`s `[features]`-Tabelle so
umstellen, dass `leo4-parser` in `default` steht;
oxilean-parse-direct bleibt als Fallback, wenn
`TranslateError::Unsupported` zuschlägt (Varianten wie
`Dsl`, `HashCommand`, `DefinitionByArms` ohne
oxilean-Äquivalent).

== OX5-oxi: Elab-Env-Bootstrap

`transpile_source_to_unit` der rust-transpile-Pipeline
rief `oxilean_elab::elaborate_decl(&env, &decl)` mit
`Environment::new()` auf. Selbst ein erfolgreich
geparstes `def x : UInt64 := 0` scheiterte dann an
`NameNotFound("UInt64")`. Fix: ein
`leo4_env_bootstrap`-Modul in `leo4-oxilean-build`,
das `oxilean_kernel::init_builtin_env` (Bool / Unit /
Empty / Nat / String / Eq / Prod / List + Axiome +
Nat-Arithmetik) aufruft und dann mit den
Grenzprimitiven ergänzt, die leo4 braucht und OxiLean
nicht ausliefert (`UInt8..128`, `Int8..128`,
`Float32`, `Float64`, `Char`) — als
`Declaration::Axiom { ty: Sort 1, … }`.

Die Augmentationsliste lebt in
`LEO4_PRIMITIVE_TYPES: &[&str]` als Single-Source. Mit
einem Regression-Guard-Test gegen
`oxilean_kernel::builtin::all_builtin_names()`
absichern, damit es laut scheitert, falls OxiLean
upstream einen der Augmentationsnamen mitliefert (sonst
stiller `DuplicateDeclaration`).

== OX5-msl: bestätigt no-op

Beim Bauen von leo4 mit dem mslean4-Backend (lean.h +
libleanshared) tritt das OX5-Problem nicht auf — das
Lake-Plugin lässt Leans eigenen Elaborator in einem
`import Lean`-Kontext laufen, so dass `UInt64` / `+`
konstruktiv sichtbar sind. Die mslean4-Hälfte des
Splits ist ein Dokumentationsartefakt, keine
Codearbeit. Die Code-Auditierung
(`grep -rn 'Environment::new\|elaborate_decl'`) zeigt,
dass jede Aufrufstelle in `sibling/leo4-oxilean-build`
liegt.

== Post-OX6 CLI-Refactor

Das Flag `--impl <kind>` der leo4-CLI bei `create` und
`init` wandert in eine per-(sub)crate-Datei
`leo4.toml`. Einen `Leo4Config`-Parser bauen (TOML,
`[[impl]]`-Arrays-of-Tables) mit Validierung
disjunkter Output-Pfade. `--subcrate` zu `create`
hinzufügen, das nach oben nach der nächsten
`[workspace]`-Cargo.toml sucht und das neue Crate
in deren `members`-Array einträgt (sowohl inline als
auch mehrzeiliges Array, idempotent). `init` bekommt
eine 3-Wege-Priorität: vorhandenes `leo4.toml` →
unangetastet; Legacy-`.leo4-impl`-Marker → migrieren
+ Marker löschen; keins → Standard
`[[impl]] kind = "mslean4"`. `run` löst das
Ziel-Impl mit 4-Wege-Priorität auf:
`leo4.toml + --impl` (Selektor) → erster `[[impl]]`
→ Legacy-Marker → harter Fehler.

== C5: musl Tier 1+ (no-mslean4-no-lake Pfade)

Wenn der Host glibc ist, Sie aber ein statisches
musl-Binary für den OxiLean-only-Transpile-Pfad
ausliefern wollen, muss nichts an leo4s Quellcode
geändert werden. Audit-verifiziert: 14
Workspace-Crates bauen unter
`--target x86_64-unknown-linux-musl` out-of-box
sauber; 2 (`leo4-rust-bridge`, `leo4-wasm`)
benötigen eine musl-C-Toolchain auf dem Host
(`musl-clang` oder `musl-gcc`). Archs
`musl-clang`-Wrapper hat eine Paketierungs-
Eigenheit — er gibt `-nostdinc` weiter, ohne den
Freistanding-Header-Pfad von Clang wiederherzustellen.
Die `build.rs` von `leo4-rust-bridge` erkennt den
Wrapper automatisch und ergänzt
`-isystem $(clang -print-resource-dir)/include`, so
dass `<stdatomic.h>` aufgelöst wird. No-op für jede
andere Toolchain.

== Leo4.Platform-Lean-Layer

Drei Einträge des OS-PORTABILITY-Ledgers innerhalb
von `lake/Leo4/Leo4/Build.lean` (.so-Extension
hartkodiert, `-Wl,-rpath` überall, `-shared`-Flag)
wandern in ein neues Modul
`lake/Leo4/Leo4/Platform.lean` — `dynlibExt`,
`dynlibPrefix`, `isPlatformDynlib`, `stemOfDynlib`,
`linkRpath?`, `defaultShimSuffix`. `Build.lean`s
`collectLibDir` und `linkShared` konsumieren die
Helfer statt hartkodierter Literale. OS-PORTABILITY-
Politik: neue per-OS-Zweige gehören in dieses Modul.

== Windows-IPC Worker-Seite

Der Windows-Zweig von `open_ipc_channel` in
`leo4-rust-worker` war ein Stub, der
`"Windows named-pipe IPC not yet implemented"`
zurückgab. Ausfüllen:
`std::fs::OpenOptions::new().read(true).write(true).open(pipe_path)`
(unter der Haube `CreateFileW`) ist das clientseitige
Pendant zu `CreateNamedPipeA` / `ConnectNamedPipe`
des Dispatchers. Einen 10×-Retry mit linearem
Backoff für das schmale Rennen einbauen, in dem der
Worker-Prozess startet, bevor der Dispatcher den
Pipe-Namen beim OS registriert hat.
