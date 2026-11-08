//! Source-proven CommonJS callable exports, with ambiguity and scope controls.

mod support;

use std::error::Error;
use std::fs;

#[test]
fn commonjs_callable_exports_keep_their_identity_and_namespace() -> Result<(), Box<dyn Error>> {
    let root = support::exclusive_dir("commonjs-callable")?;
    let cases = [
        (
            "named-expression",
            "module.exports = function selected() {};\n",
            true,
        ),
        (
            "conditional-expression",
            "if (enabled) module.exports = function selected() {};\n",
            false,
        ),
        (
            "reassigned-expression",
            "module.exports = function selected() {};\nmodule.exports = other;\n",
            false,
        ),
        (
            "direct",
            "function selected() {}\nmodule.exports = selected;\n",
            true,
        ),
        (
            "chain",
            "function selected() {}\nexports = module.exports = selected;\n",
            true,
        ),
        (
            "prototype",
            "function selected() {}\nmodule.exports = selected;\nselected.prototype.method = function method() {};\n",
            true,
        ),
        (
            "conditional",
            "function selected() {}\nif (enabled) module.exports = selected;\n",
            false,
        ),
        (
            "shadowed",
            "const module = {};\nfunction selected() {}\nmodule.exports = selected;\n",
            false,
        ),
        (
            "reassigned",
            "function selected() {}\nmodule.exports = selected;\nmodule.exports = other;\n",
            false,
        ),
        (
            "mutated",
            "function selected() {}\nselected = other;\nmodule.exports = selected;\n",
            false,
        ),
        (
            "nested",
            "function selected() {}\nfunction set() { module.exports = selected; }\n",
            false,
        ),
        (
            "esm",
            "export function selected() {}\nmodule.exports = selected;\n",
            false,
        ),
        (
            "module-destructured",
            "function selected() {}\nconst {module} = custom;\nmodule.exports = selected;\n",
            false,
        ),
        (
            "incremented",
            "function selected() {}\nmodule.exports = selected;\nmodule.exports++;\n",
            false,
        ),
        (
            "augmented",
            "function selected() {}\nmodule.exports = selected;\nmodule.exports += other;\n",
            false,
        ),
        (
            "deleted",
            "function selected() {}\nmodule.exports = selected;\ndelete module.exports;\n",
            false,
        ),
        (
            "pattern-write",
            "function selected() {}\n[selected] = other;\nmodule.exports = selected;\n",
            false,
        ),
        (
            "loop-write",
            "function selected() {}\nfor (selected of other) {}\nmodule.exports = selected;\n",
            false,
        ),
        (
            "export-loop-write",
            "function selected() {}\nmodule.exports = selected;\nfor (module.exports of other) {}\n",
            false,
        ),
        (
            "export-pattern-write",
            "function selected() {}\nmodule.exports = selected;\n[module.exports] = other;\n",
            false,
        ),
        (
            "module-escape",
            "function selected() {}\nmodule.exports = selected;\nconst escaped = {module};\nescaped.module.exports = other;\n",
            false,
        ),
    ];
    for (name, source, expected) in cases {
        let directory = root.join(name);
        fs::create_dir(&directory)?;
        fs::write(directory.join("provider.js"), source)?;
        fs::write(
            directory.join("main.js"),
            "const renamed = require('./provider');\nfunction caller() { renamed(); }\n",
        )?;
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &root)
            .args(["show", "caller", "--exact", "-C"])
            .arg(&directory)
            .output()?;
        let text = String::from_utf8(output.stdout)?;
        assert!(output.status.success(), "{name}: {text}");
        assert_eq!(text.contains("> selected "), expected, "{name}: {text}");
    }
    let directory = root.join("expression-name-scope");
    fs::create_dir(&directory)?;
    fs::write(
        directory.join("provider.js"),
        "module.exports = function selected() {};\nfunction unrelated() { selected(); }\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &root)
        .args(["show", "unrelated", "--exact", "-C"])
        .arg(&directory)
        .output()?;
    let text = String::from_utf8(output.stdout)?;
    assert!(output.status.success(), "{text}");
    assert!(!text.contains("selected"), "{text}");
    let directory = root.join("expression-not-a-named-export");
    fs::create_dir(&directory)?;
    fs::write(
        directory.join("provider.js"),
        "module.exports = function selected() {};\n",
    )?;
    fs::write(
        directory.join("main.js"),
        "import { selected } from './provider.js';\nfunction caller() { selected(); }\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &root)
        .args(["show", "caller", "--exact", "-C"])
        .arg(&directory)
        .output()?;
    let text = String::from_utf8(output.stdout)?;
    assert!(output.status.success(), "{text}");
    assert!(!text.contains("selected"), "{text}");
    let directory = root.join("embedded-expression-name-scope");
    fs::create_dir(&directory)?;
    fs::write(
        directory.join("page.html"),
        "<h1>Guide</h1>\n<script>module.exports = function selected() {};</script>\n<script>function unrelated() { selected(); }</script>\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &root)
        .args(["show", "unrelated", "--exact", "-C"])
        .arg(&directory)
        .output()?;
    let text = String::from_utf8(output.stdout)?;
    assert!(output.status.success(), "{text}");
    assert!(!text.contains("selected"), "{text}");
    let directory = root.join("expression-is-not-a-proto-handler-by-name");
    fs::create_dir(&directory)?;
    fs::write(
        directory.join("provider.js"),
        "module.exports = function charge() {};\n",
    )?;
    fs::write(
        directory.join("payment.proto"),
        "syntax = \"proto3\";\nservice Payment {\n  rpc Charge(Request) returns (Response);\n}\nmessage Request {}\nmessage Response {}\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &root)
        .args(["show", "charge", "--json", "-C"])
        .arg(&directory)
        .output()?;
    let text = String::from_utf8(output.stdout)?;
    assert!(output.status.success(), "{text}");
    assert!(!text.contains("\"kind\":\"implements\""), "{text}");
    let directory = root.join("forward");
    fs::create_dir(&directory)?;
    fs::write(
        directory.join("provider.js"),
        "function selected() {}\nfunction decoy() {}\nexports = module.exports = selected;\nexports.extra = function extra() {};\n",
    )?;
    fs::write(
        directory.join("index.js"),
        "module.exports = require('./provider');\n",
    )?;
    fs::write(
        directory.join("main.js"),
        "const renamed = require('./index');\nconst namespace = require('./provider');\nfunction caller() { renamed(); namespace.extra(); renamed.decoy(); }\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &root)
        .args(["show", "caller", "--exact", "-C"])
        .arg(&directory)
        .output()?;
    let text = String::from_utf8(output.stdout)?;
    assert!(output.status.success(), "{text}");
    assert!(text.contains("> selected provider.js:1"), "{text}");
    assert!(text.contains("> extra "), "{text}");
    assert!(!text.contains("> decoy "), "{text}");
    for (name, main, package) in [
        (
            "var",
            "var renamed = require('./provider');\nfunction caller() { renamed(); }\n",
            "{}",
        ),
        (
            "shadow-require",
            "function require(path) { return function fake() {}; }\nvar renamed = require('./provider');\nfunction caller() { renamed(); }\n",
            "{}",
        ),
        (
            "pattern-require",
            "const {require} = custom;\nvar renamed = require('./provider');\nfunction caller() { renamed(); }\n",
            "{}",
        ),
        (
            "module-package",
            "var renamed = require('./provider');\nfunction caller() { renamed(); }\n",
            "{\"type\":\"module\"}",
        ),
        (
            "reassigned-alias",
            "var renamed = require('./provider');\nrenamed = other;\nfunction caller() { renamed(); }\n",
            "{}",
        ),
        (
            "conditional-alias",
            "if (enabled) { var renamed = require('./provider'); }\nfunction caller() { renamed(); }\n",
            "{}",
        ),
        (
            "parameter-shadow",
            "var renamed = require('./provider');\nfunction caller(renamed) { renamed(); }\n",
            "{}",
        ),
    ] {
        let directory = root.join(name);
        fs::create_dir(&directory)?;
        fs::write(directory.join("package.json"), package)?;
        fs::write(
            directory.join("provider.js"),
            "function selected() {}\nmodule.exports = selected;\n",
        )?;
        fs::write(directory.join("main.js"), main)?;
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &root)
            .args(["show", "caller", "--exact", "-C"])
            .arg(&directory)
            .output()?;
        let text = String::from_utf8(output.stdout)?;
        assert!(output.status.success(), "{name}: {text}");
        assert_eq!(
            text.contains("> selected "),
            name == "var",
            "{name}: {text}"
        );
    }
    for (name, source, expected) in [
        (
            "forward-literal",
            "module.exports = require('./leaf');\n",
            true,
        ),
        (
            "forward-shadow",
            "function require() { return other; }\nmodule.exports = require('./leaf');\n",
            false,
        ),
        (
            "forward-cycle",
            "module.exports = require('./provider');\n",
            false,
        ),
    ] {
        let directory = root.join(name);
        fs::create_dir(&directory)?;
        fs::write(directory.join("provider.js"), source)?;
        fs::write(
            directory.join("leaf.js"),
            "function selected() {}\nmodule.exports = selected;\n",
        )?;
        fs::write(
            directory.join("main.js"),
            "var renamed = require('./provider');\nfunction caller() { renamed(); }\n",
        )?;
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &root)
            .args(["show", "caller", "--exact", "-C"])
            .arg(&directory)
            .output()?;
        let text = String::from_utf8(output.stdout)?;
        assert!(output.status.success(), "{name}: {text}");
        assert_eq!(text.contains("> selected "), expected, "{name}: {text}");
    }
    fs::remove_dir_all(root)?;
    Ok(())
}
