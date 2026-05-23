/- Leo4Rust — empty marker library.
   Importing this module pulls in `Leo4Rust`'s `extern_lib`s
   transitively via Lake's package-dep graph (Lake/Build/
   Executable.lean walks `dep.externLibs` on every required
   package, not just imported modules). The empty body keeps
   the Lean side free of any reverse-direction surface; users
   import the *generated* wrapper module
   (`<pkg>.leo4-rust-imports.lean`) for the actual typed API.
-/
namespace Leo4Rust
end Leo4Rust
