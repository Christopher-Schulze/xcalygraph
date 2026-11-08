//! Callback values at module scope remain uses, without fabricated invocation edges.

mod support;

use std::error::Error;
use std::fs;
use std::path::PathBuf;

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn python_list_callbacks_keep_lexical_identity() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("python-list-callback")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("callbacks.py"),
        "def _shadowed(): pass\ndef use_parameter(_shadowed):\n    dispatch([_shadowed])\ndef use_assignment():\n    _shadowed = 1\n    dispatch([_shadowed])\ndef use_nested():\n    def _passed(): pass\n    dispatch([_passed])\ndef assignment_after_use():\n    dispatch([_shadowed])\n    _shadowed = 2\ndef typed_parameter(_shadowed: int):\n    dispatch([_shadowed])\ndef default_parameter(_shadowed=1):\n    dispatch([_shadowed])\ndef tuple_assignment():\n    _shadowed, other = (1, 2)\n    dispatch([_shadowed])\ndef loop_binding():\n    for _shadowed in []:\n        dispatch([_shadowed])\ndef enclosing_binding():\n    _shadowed = 1\n    def inner():\n        dispatch([_shadowed])\ndef import_binding():\n    import os as _shadowed\n    dispatch([_shadowed])\ndef variadic_binding(*_shadowed):\n    dispatch([_shadowed])\ndef outer_scope():\n    _inner = 1\n    def inner_scope():\n        def _inner(): pass\n        dispatch([_inner])\ndef _decorator(): pass\n@mark([_decorator])\ndef decorated(): pass\ndef lambda_binding():\n    return lambda _shadowed: [_shadowed]\ndef comprehension_binding():\n    return [[_shadowed] for _shadowed in []]\ndef _lambda_outer(): pass\ndef valid_lambda():\n    f = lambda _lambda_outer=1: [_lambda_outer]\n    dispatch([_lambda_outer])\ndef _default_value(): pass\ndef outer_default():\n    return lambda _default_value=[_default_value]: _default_value\ndef _ordinary_default(): pass\ndef use_default(_ordinary_default=[_ordinary_default]): pass\n",
    )?;
    for (name, expected_line) in [
        ("_passed", Some(9)),
        ("_shadowed", None),
        ("_inner", Some(36)),
        ("_decorator", Some(38)),
        ("_lambda_outer", Some(47)),
        ("_default_value", Some(50)),
        ("_ordinary_default", Some(52)),
    ] {
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args(["show", name, "--json", "--exact", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(fixture.0.join("index"))
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout)?;
        let incoming: Vec<_> = text
            .lines()
            .filter(|line| line.contains("\"rel\":\"in\""))
            .collect();
        if name == "_decorator" {
            assert!(
                incoming.iter().all(|edge| !edge.contains("decorated")),
                "{text}"
            );
        }
        match expected_line {
            Some(line) => assert!(
                incoming
                    .iter()
                    .any(|edge| edge.contains("\"kind\":\"uses\"")
                        && edge.contains(&format!("\"line\":{line},"))),
                "{text}"
            ),
            None => assert!(incoming.is_empty(), "{text}"),
        }
    }
    Ok(())
}

#[test]
fn python_global_list_callback_does_not_bind_to_an_enclosing_function() -> Result<(), Box<dyn Error>>
{
    let fixture = Fixture(support::exclusive_dir("python-list-global")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("callbacks.py"),
        "def target(): pass\ndef outer():\n    def target(): pass\n    def inner():\n        global target\n        dispatch([target])\n    def other():\n        nonlocal target\n        dispatch([target])\ndef change():\n    global target\n    target = 1\n    dispatch([target])\ndef replaced(): pass\nreplaced = 1\ndef use_replaced():\n    global replaced\n    dispatch([replaced])\n",
    )?;
    for (location, expected) in [("callbacks.py:1", true), ("callbacks.py:3", false)] {
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args(["show", location, "--json", "--exact", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(fixture.0.join("index"))
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout)?;
        let has_use = text.lines().any(|line| {
            line.contains("\"rel\":\"in\"")
                && line.contains("\"kind\":\"uses\"")
                && line.contains("\"line\":6,")
        });
        assert_eq!(has_use, expected, "{text}");
        let nonlocal_use = text.lines().any(|line| {
            line.contains("\"rel\":\"in\"")
                && line.contains("\"kind\":\"uses\"")
                && line.contains("\"line\":9,")
        });
        assert_eq!(nonlocal_use, !expected, "{text}");
        assert!(
            text.lines().all(|line| !line.contains("\"line\":13,")),
            "{text}"
        );
    }
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "callbacks.py:14", "--json", "--exact", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert!(
        text.lines().all(|line| !line.contains("\"rel\":\"in\"")),
        "{text}"
    );
    Ok(())
}

#[test]
fn module_callback_values_are_used_without_calling_or_unshadowing_them()
-> Result<(), Box<dyn Error>> {
    for (extension, source, site) in [
        (
            "js",
            "function _passed() { return 1; }\nfunction _shadowed() { return 2; }\nfunction _unused() { return 3; }\nfunction other(_shadowed) { register(_shadowed); }\nregister(_passed);\n",
            5,
        ),
        (
            "ts",
            "function _passed() { return 1; }\nfunction _shadowed() { return 2; }\nfunction _unused() { return 3; }\nfunction other(_shadowed) { register(_shadowed); }\nregister(_passed);\n",
            5,
        ),
        // Python parameters are not recorded as bindings (pytest injects fixtures through
        // them), so its case has no shadowing parameter.
        (
            "py",
            "def _passed(): return 1\ndef _shadowed(): return 2\ndef _unused(): return 3\nregister(_passed)\n",
            4,
        ),
    ] {
        let fixture = Fixture(support::exclusive_dir("callback-values")?);
        let root = fixture.0.join("repo");
        fs::create_dir(&root)?;
        let path = format!("callbacks.{extension}");
        fs::write(root.join(&path), source)?;
        for name in ["_passed", "_shadowed", "_unused"] {
            let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
                .args(["show", name, "--json", "--exact", "-C"])
                .arg(&root)
                .arg("--index-dir")
                .arg(fixture.0.join("index"))
                .output()?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout)?;
            let incoming: Vec<_> = text
                .lines()
                .filter(|line| line.contains("\"rel\":\"in\""))
                .collect();
            if name == "_passed" {
                assert_eq!(incoming.len(), 1, "{extension}: {text}");
                assert!(
                    incoming[0].contains("\"kind\":\"uses\"")
                        && incoming[0].contains(&format!("\"path\":\"{path}\""))
                        && incoming[0].contains(&format!("\"line\":{site},")),
                    "{text}"
                );
            } else {
                assert!(incoming.is_empty(), "{extension} {name}: {text}");
            }
        }
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args(["audit", "--json", "--exact", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(fixture.0.join("index"))
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout)?;
        for (name, dead) in [("_passed", false), ("_shadowed", true), ("_unused", true)] {
            let found = text.lines().any(|line| {
                line.contains("\"rel\":\"dead\"")
                    && line.contains(&format!("\"name\":\"callbacks.{name}\""))
            });
            assert_eq!(found, dead, "{extension} {name}: {text}");
        }
    }
    Ok(())
}

#[test]
fn anonymous_callback_parameters_do_not_shadow_outer_middleware_uses() -> Result<(), Box<dyn Error>>
{
    for extension in [
        "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "ets", "JS",
    ] {
        let fixture = Fixture(support::exclusive_dir("anonymous-callback-scope")?);
        let root = fixture.0.join("repo");
        fs::create_dir(&root)?;
        fs::write(
            root.join(format!("callbacks.{extension}")),
            "function middleware() { return 1; }\nregister(function(middleware) { consume(middleware); });\nregister(middleware);\n",
        )?;
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args(["show", "middleware", "--json", "--exact", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(fixture.0.join("index"))
            .output()?;
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)?;
        let incoming: Vec<_> = text
            .lines()
            .filter(|line| line.contains("\"rel\":\"in\""))
            .collect();
        assert_eq!(incoming.len(), 1, "{extension}: {text}");
        assert!(
            incoming[0].contains("\"kind\":\"uses\"") && incoming[0].contains("\"line\":3,"),
            "{extension}: {text}"
        );
    }
    Ok(())
}

#[test]
fn html_private_function_is_not_visible_in_another_script() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("html-script-isolation")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("callbacks.html"),
        "<script>\nfunction visible() {}\n(function(hidden) {})(null);\nhidden();\n</script>\n<script>\n(function() { function hidden() {} })();\nvisible();\n</script>\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "hidden", "--json", "--exact", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(
        !text.lines().any(|line| line.contains("\"rel\":\"in\"")),
        "{text}"
    );
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "visible", "--json", "--exact", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(
        text.lines()
            .any(|line| line.contains("\"rel\":\"in\"") && line.contains("\"line\":8,")),
        "{text}"
    );
    Ok(())
}

#[test]
fn html_script_local_function_identity_survives_document_merge() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("html-callback-identity")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("callbacks.html"),
        "<h1>Header</h1>\n<script>function first() {}</script>\n<script>\nfunction middleware() {}\nregister(function(middleware) {\n function middleware() {}\n function inside() { middleware(); }\n inside();\n});\n</script>\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "callbacks.html:6", "--json", "--exact", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(
        text.lines().any(|line| line.contains("\"rel\":\"in\"")
            && line.contains("\"line\":7,")
            && line.contains("\"kind\":\"calls\"")),
        "{text}"
    );
    Ok(())
}

#[test]
fn callback_bindings_do_not_leak_between_sibling_functions() -> Result<(), Box<dyn Error>> {
    for extension in ["js", "ts"] {
        let fixture = Fixture(support::exclusive_dir("sibling-callback-scope")?);
        let root = fixture.0.join("repo");
        fs::create_dir(&root)?;
        fs::write(
            root.join(format!("callbacks.{extension}")),
            "function middleware() {}\nregister(function(middleware) { function inside() { consume(middleware); } inside(); });\nregister(function() { function middleware() {} consume(middleware); });\nregister(middleware);\n",
        )?;
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args([
                "show",
                &format!("callbacks.{extension}:1"),
                "--json",
                "--exact",
                "-C",
            ])
            .arg(&root)
            .arg("--index-dir")
            .arg(fixture.0.join("index"))
            .output()?;
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)?;
        let incoming: Vec<_> = text
            .lines()
            .filter(|line| line.contains("\"rel\":\"in\""))
            .collect();
        assert_eq!(incoming.len(), 1, "{extension}: {text}");
        assert!(incoming[0].contains("\"line\":4,"), "{extension}: {text}");
    }
    Ok(())
}

#[test]
fn callback_inner_function_can_shadow_a_parameter() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("callback-inner-declaration")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("callbacks.js"),
        "function middleware() {}\nregister(function(middleware) {\n function middleware() {}\n function inside() { middleware(); }\n function inside2() { new middleware(); }\n inside(); inside2();\n});\nregister(middleware);\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "callbacks.js:3", "--json", "--exact", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(
        text.lines().any(|line| line.contains("\"rel\":\"in\"")
            && line.contains("\"line\":4,")
            && line.contains("\"kind\":\"calls\"")),
        "{text}"
    );
    assert!(
        text.lines().any(|line| line.contains("\"rel\":\"in\"")
            && line.contains("\"line\":5,")
            && line.contains("\"kind\":\"calls\"")),
        "{text}"
    );
    Ok(())
}

#[test]
fn callback_parameter_shadows_outer_function_inside_nested_named_function()
-> Result<(), Box<dyn Error>> {
    for extension in ["js", "ts"] {
        let fixture = Fixture(support::exclusive_dir("nested-callback-scope")?);
        let root = fixture.0.join("repo");
        fs::create_dir(&root)?;
        fs::write(
            root.join(format!("callbacks.{extension}")),
            "function middleware() {}\nregister(function(middleware) { function inside() { consume(middleware); } inside(); });\nregister(middleware);\n",
        )?;
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args(["show", "middleware", "--json", "--exact", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(fixture.0.join("index"))
            .output()?;
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)?;
        let incoming: Vec<_> = text
            .lines()
            .filter(|line| line.contains("\"rel\":\"in\""))
            .collect();
        assert_eq!(incoming.len(), 1, "{extension}: {text}");
        assert!(incoming[0].contains("\"line\":3,"), "{extension}: {text}");
    }
    Ok(())
}

#[test]
fn named_function_expression_does_not_reference_an_outer_function() -> Result<(), Box<dyn Error>> {
    for extension in ["js", "ts"] {
        let fixture = Fixture(support::exclusive_dir("named-callback-scope")?);
        let root = fixture.0.join("repo");
        fs::create_dir(&root)?;
        fs::write(
            root.join(format!("callbacks.{extension}")),
            "function middleware() {}\nregister(function middleware() { consume(middleware); });\nregister(middleware);\n",
        )?;
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args(["show", "middleware", "--json", "--exact", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(fixture.0.join("index"))
            .output()?;
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)?;
        let incoming: Vec<_> = text
            .lines()
            .filter(|line| line.contains("\"rel\":\"in\""))
            .collect();
        assert_eq!(incoming.len(), 1, "{extension}: {text}");
        assert!(incoming[0].contains("\"line\":3,"), "{extension}: {text}");
    }
    Ok(())
}

#[test]
fn embedded_script_callbacks_keep_their_own_scope() -> Result<(), Box<dyn Error>> {
    for (extension, source) in [
        (
            "vue",
            "<script>\nfunction middleware() {}\nregister(function(middleware) { consume(middleware); });\nregister(middleware);\n</script>\n",
        ),
        (
            "svelte",
            "<script>\nfunction middleware() {}\nregister(function(middleware) { consume(middleware); });\nregister(middleware);\n</script>\n",
        ),
        (
            "astro",
            "---\nfunction middleware() {}\nregister(function(middleware) { consume(middleware); });\nregister(middleware);\n---\n",
        ),
        (
            "html",
            "<script>\nfunction middleware() {}\nregister(function(middleware) { consume(middleware); });\nregister(middleware);\n</script>\n",
        ),
    ] {
        let fixture = Fixture(support::exclusive_dir("embedded-callback-scope")?);
        let root = fixture.0.join("repo");
        fs::create_dir(&root)?;
        fs::write(root.join(format!("callbacks.{extension}")), source)?;
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args(["show", "middleware", "--json", "--exact", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(fixture.0.join("index"))
            .output()?;
        assert!(
            output.status.success(),
            "{extension}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout)?;
        let incoming: Vec<_> = text
            .lines()
            .filter(|line| line.contains("\"rel\":\"in\""))
            .collect();
        assert_eq!(incoming.len(), 1, "{extension}: {text}");
        assert!(incoming[0].contains("\"line\":4,"), "{extension}: {text}");
    }
    Ok(())
}

#[test]
fn loop_bindings_do_not_name_an_outer_function() -> Result<(), Box<dyn Error>> {
    for extension in ["js", "ts"] {
        for source in [
            "function middleware() {}\nregister(() => { for (const middleware of [1]) consume(middleware); });\nregister(middleware);\n",
            "function middleware() {}\nregister(() => { for (const k of [1]) { var middleware = k; } consume(middleware); });\nregister(middleware);\n",
        ] {
            let fixture = Fixture(support::exclusive_dir("callback-loop-scope")?);
            let root = fixture.0.join("repo");
            fs::create_dir(&root)?;
            fs::write(root.join(format!("callbacks.{extension}")), source)?;
            let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
                .args(["show", "middleware", "--json", "--exact", "-C"])
                .arg(&root)
                .arg("--index-dir")
                .arg(fixture.0.join("index"))
                .output()?;
            assert!(output.status.success());
            let text = String::from_utf8(output.stdout)?;
            let incoming: Vec<_> = text
                .lines()
                .filter(|line| line.contains("\"rel\":\"in\""))
                .collect();
            assert_eq!(incoming.len(), 1, "{extension}: {text}");
            assert!(incoming[0].contains("\"line\":3,"), "{extension}: {text}");
        }
    }
    Ok(())
}

#[test]
fn values_without_recorded_local_bindings_do_not_name_same_file_functions()
-> Result<(), Box<dyn Error>> {
    // Lua extraction records no local bindings, so `f` at line 2 may be the local of line 1
    // rather than the function of line 3: no `uses` edge is claimed.
    let fixture = Fixture(support::exclusive_dir("unrecorded-binding-values")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("values.lua"),
        "local f = load(\"return 1\")\nregister(f)\nfunction f() return 2 end\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "f", "--json", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert!(
        !text
            .lines()
            .any(|line| line.contains("\"rel\":\"in\"") && line.contains("\"kind\":\"uses\"")),
        "{text}"
    );
    Ok(())
}

#[test]
fn pascal_callbacks_with_required_arguments_are_values() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("pascal-callback-values")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("callbacks.pas"),
        include_bytes!("fixtures/pascal-callback-values.pas"),
    )?;
    for (name, kind, site) in [("Handler", "uses", 12), ("Answer", "calls", 13)] {
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args(["show", name, "--exact", "--json", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(fixture.0.join("index"))
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout)?;
        let incoming: Vec<_> = text
            .lines()
            .filter(|row| row.contains("\"rel\":\"in\""))
            .collect();
        assert_eq!(incoming.len(), 1, "{text}");
        assert!(
            incoming[0].contains(&format!("\"kind\":\"{kind}\""))
                && incoming[0].contains(&format!("\"line\":{site},")),
            "{text}"
        );
    }
    Ok(())
}

#[test]
fn python_default_values_are_uses_of_the_enclosing_binding() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("python-default-values")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("defaults.py"),
        "def _target(): return 1\ndef plain(callback=_target): return callback\ndef typed(callback: object = _target): return callback\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "_target", "--exact", "--json", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    for (name, line) in [("defaults.plain", 2), ("defaults.typed", 3)] {
        assert!(
            text.lines().any(|row| row.contains("\"rel\":\"in\"")
                && row.contains(&format!("\"name\":\"{name}\""))
                && row.contains(&format!("\"line\":{line},"))
                && row.contains("\"kind\":\"uses\"")),
            "{text}"
        );
    }
    Ok(())
}
