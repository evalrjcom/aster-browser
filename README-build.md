# Aster Browser Windows build

Prototype Rust/FLTK browser. HTTPS requests use reqwest with rustls. HTML rendering is deliberately limited and JavaScript/QuickJS-NG is not integrated yet.

The Windows GitHub Actions workflow builds on pushes to main and feature branches and on pull requests. Open Actions > Build Windows EXE and download the aster-browser-windows-x64 artifact after a successful build.

To build locally: cargo build --release

This is not a complete HTML/CSS/JavaScript browser and must not be described as standards compliant.
