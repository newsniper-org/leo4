// docs/template/leo4-book.typ — shared Typst template for the leo4
// learning material and implement-from-scratch guide book. Used by
// all four language editions (en / ko / ja / de) of both documents.
//
// Typst version: requires >= 0.14 (uses `set heading numbering`,
// `outline` defaults from 0.14, `figure` placement updates).
//
// Usage in a doc file:
//
//   #import "../../template/leo4-book.typ": book
//   #show: book.with(
//     title: "leo4 — Learning Material",
//     subtitle: "English Edition",
//     author: "윤병익",
//     lang: "en",
//   )
//   = First chapter
//   ...

#let book(
  title: "leo4",
  subtitle: none,
  author: "leo4 contributors",
  lang: "en",
  body
) = {
  set document(title: title, author: author)
  set page(
    paper: "a4",
    numbering: "1",
    number-align: center,
    margin: (top: 2.5cm, bottom: 2.5cm, left: 2.5cm, right: 2.5cm),
  )
  set text(
    font: ("New Computer Modern", "Noto Sans CJK KR", "Noto Sans CJK JP",
           "DejaVu Sans"),
    size: 11pt,
    lang: lang,
  )
  set par(justify: true, leading: 0.7em)
  set heading(numbering: "1.1.1.")
  show heading.where(level: 1): it => {
    pagebreak(weak: true)
    set text(size: 22pt, weight: "bold")
    block(below: 1.2em, it)
  }
  show heading.where(level: 2): it => {
    set text(size: 16pt, weight: "bold")
    block(above: 1.5em, below: 0.8em, it)
  }
  show heading.where(level: 3): it => {
    set text(size: 13pt, weight: "bold")
    block(above: 1.2em, below: 0.5em, it)
  }
  show raw.where(block: true): it => {
    set text(font: "DejaVu Sans Mono", size: 9pt)
    block(
      fill: rgb("#f5f5f5"),
      inset: 8pt,
      radius: 3pt,
      width: 100%,
      it,
    )
  }
  show raw.where(block: false): it => {
    set text(font: "DejaVu Sans Mono", size: 9.5pt)
    box(fill: rgb("#f0f0f0"), inset: (x: 3pt, y: 0pt), outset: (y: 2pt),
      radius: 2pt, it)
  }
  // Cover page.
  align(center + horizon)[
    #text(size: 30pt, weight: "bold")[#title]
    #v(1em)
    #if subtitle != none {
      text(size: 16pt)[#subtitle]
    }
    #v(2em)
    #text(size: 14pt)[#author]
    #v(0.5em)
    #text(size: 11pt, fill: gray)[leo4 project documentation]
  ]
  pagebreak()
  // Table of contents.
  outline(title: [Contents], indent: auto)
  pagebreak()
  body
}
