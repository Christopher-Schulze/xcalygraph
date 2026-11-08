//! A declared type proves a receiver only when it names one value type.

mod support;

use std::error::Error;
use std::fs;
use std::path::Path;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

type TestResult = Result<(), Box<dyn Error>>;

/// `name conf` of the outgoing edges of `symbol`, `conf` empty when exact.
fn calls(root: &Path, symbol: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let output = support::command(BINARY, root)
        .args(["show", symbol, "--json", "-C"])
        .arg(root)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    let mut found = Vec::new();
    for line in String::from_utf8(output.stdout)?.lines() {
        if !line.contains("\"t\":\"edge\"") || !line.contains("\"rel\":\"out\"") {
            continue;
        }
        let field = |key: &str| {
            line.split(&format!("\"{key}\":\""))
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_default()
                .to_owned()
        };
        if field("kind") == "calls" {
            found.push(format!("{} {}", field("name"), field("conf")));
        }
    }
    found.sort();
    Ok(found)
}

const TS: &str = "export class Left { run() { return 1 } }\n\
export class Right { run() { return 2 } }\n\
export class Only { solo() { return 3 } }\n\
export function direct(x: Left | Right) { x.run(); }\n\
export function nullable(x: Left | null) { x.run(); }\n\
export function leading(x: undefined | Right) { x?.run(); }\n\
export function both(x: Left & Only) { x.solo(); }\n\
export function partial(x: Left | Only) { x.solo(); }\n\
export function elements(items: (Left | Right)[]) { for (const x of items) { x.run(); } }\n";

#[test]
fn python_import_aliases_obey_lexical_scope_and_branches() -> TestResult {
    let root = support::exclusive_dir("python-import-scope")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(root.join("right.py"), "def right(): pass\n")?;
    fs::write(
        root.join("app.py"),
        "try:\n    from left import left as optional_call\nexcept ImportError:\n    optional_call = None\n\ndef optional():\n    optional_call()\n\ndef use_left():\n    from left import left as call\n    call()\n\ndef use_right():\n    from right import right as call\n    call()\n\ndef branches(flag):\n    if flag:\n        from left import left as call\n        call()\n    else:\n        from right import right as call\n        call()\n    call()\n\ndef try_return():\n    try:\n        from left import left as call\n    except ImportError:\n        return\n    call()\n\ndef try_fallthrough():\n    try:\n        from left import left as call\n    except ImportError:\n        pass\n    call()\n\ndef try_finally():\n    try:\n        from left import left as call\n    except ImportError:\n        return\n    finally:\n        pass\n    call()\n",
    )?;
    fs::write(
        root.join("negative.py"),
        "try:\n    from left import left as alternative\nexcept ImportError:\n    from right import right as alternative\n\ndef fallback_case():\n    alternative()\n\ntry:\n    from left import left as overwritten\nexcept ImportError:\n    overwritten = None\noverwritten = lambda: None\n\ndef unsafe_case():\n    overwritten()\n\ndef local_fallback():\n    try:\n        from left import left as local\n    except ImportError:\n        local = lambda: None\n    local()\n",
    )?;
    fs::write(
        root.join("multi.py"),
        "try:\n    from left import left as first\n    from right import right as second\nexcept ImportError:\n    first = second = None\n\ndef chained():\n    first()\n    second()\n\ntry:\n    from left import left as third\n    from right import right as fourth\nexcept ImportError:\n    third = None\n    fourth = None\n\ndef separate():\n    third()\n    fourth()\n",
    )?;
    fs::write(
        root.join("fallback_alias.py"),
        "from right import right as fallback\ntry:\n    from left import left as selected\nexcept ImportError:\n    selected = fallback\ndef call_fallback():\n    selected()\ndef shadow_fallback(selected):\n    selected()\n",
    )?;
    assert_eq!(
        calls(&root, "call_fallback")?,
        ["left.left inferred", "right.right inferred"]
    );
    assert!(
        !calls(&root, "shadow_fallback")?
            .iter()
            .any(|edge| edge.starts_with("left.left ") || edge.starts_with("right.right "))
    );
    for (file, changed, caller) in [
        (
            "fallback_rebound.py",
            "fallback = lambda: None",
            "fallback_rebound",
        ),
        (
            "target_rebound.py",
            "selected = lambda: None",
            "target_rebound",
        ),
    ] {
        fs::write(
            root.join(file),
            format!(
                "from right import right as fallback\ntry:\n    from left import left as selected\nexcept ImportError:\n    selected = fallback\n{changed}\ndef {caller}():\n    selected()\n"
            ),
        )?;
        assert!(
            !calls(&root, caller)?
                .iter()
                .any(|edge| edge.starts_with("left.left "))
        );
    }
    assert_eq!(calls(&root, "use_left")?, ["left.left exact"]);
    assert_eq!(calls(&root, "use_right")?, ["right.right exact"]);
    assert_eq!(calls(&root, "try_return")?, ["left.left exact"]);
    assert_eq!(calls(&root, "optional")?, ["left.left inferred"]);
    assert_eq!(
        calls(&root, "chained")?,
        ["left.left inferred", "right.right inferred"]
    );
    assert_eq!(
        calls(&root, "separate")?,
        ["left.left inferred", "right.right inferred"]
    );
    assert!(
        !calls(&root, "fallback_case")?
            .iter()
            .any(|edge| edge.ends_with(" exact"))
    );
    assert!(
        !calls(&root, "unsafe_case")?
            .iter()
            .any(|edge| edge.starts_with("left.left "))
    );
    assert!(
        !calls(&root, "local_fallback")?
            .iter()
            .any(|edge| edge.starts_with("left.left "))
    );
    assert_eq!(calls(&root, "try_fallthrough")?, ["left.left inferred"]);
    assert!(
        !calls(&root, "try_finally")?
            .iter()
            .any(|edge| edge.ends_with(" exact"))
    );
    let branch_calls = calls(&root, "branches")?;
    assert!(
        branch_calls.contains(&"left.left exact".to_owned()),
        "{branch_calls:?}"
    );
    assert!(
        branch_calls.contains(&"right.right exact".to_owned()),
        "{branch_calls:?}"
    );
    assert_eq!(branch_calls.len(), 2, "{branch_calls:?}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_local_imports_do_not_become_module_exports() -> TestResult {
    let root = support::exclusive_dir("python-local-import-export")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(
        root.join("facade.py"),
        "from left import left as public\nvalue = 1; from left import left as another\n\ndef inner():\n    from left import (\n        left as exposed,\n    )\n    exposed()\n\ndef same_line(): from left import left as same_exposed\n\nclass Example:\n    from left import left as class_exposed\n",
    )?;
    fs::write(
        root.join("app.py"),
        "from facade import exposed, class_exposed, same_exposed, public, another\ndef caller():\n    exposed()\n    class_exposed()\n    same_exposed()\n\ndef valid():\n    public()\n\ndef valid_semicolon():\n    another()\n",
    )?;
    let found = calls(&root, "caller")?;
    assert!(
        !found.iter().any(|edge| edge.starts_with("left.left ")),
        "a function-local import is not a module export: {found:?}"
    );
    assert_eq!(calls(&root, "valid")?, ["left.left exact"]);
    assert_eq!(calls(&root, "valid_semicolon")?, ["left.left exact"]);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_module_aliases_do_not_cross_function_scopes() -> TestResult {
    let root = support::exclusive_dir("python-module-import-scope")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(root.join("right.py"), "def left(): pass\n")?;
    fs::write(
        root.join("app.py"),
        "def use_left():\n    import left as lib\n    lib.left()\n\ndef use_right():\n    import right as lib\n    lib.left()\n",
    )?;
    assert_eq!(calls(&root, "use_left")?, ["left.left exact"]);
    assert_eq!(calls(&root, "use_right")?, ["right.left exact"]);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_reassignment_shadows_a_local_import() -> TestResult {
    let root = support::exclusive_dir("python-import-reassignment")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(
        root.join("app.py"),
        "def caller():\n    from left import left as call\n    call = lambda: None\n    call()\n",
    )?;
    let found = calls(&root, "caller")?;
    assert!(
        !found.iter().any(|edge| edge == "left.left exact"),
        "an overwritten import cannot prove the call target: {found:?}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_parameter_shadows_a_module_import() -> TestResult {
    let root = support::exclusive_dir("python-import-parameter")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(
        root.join("app.py"),
        "from left import left as call\ndef caller(call):\n    call()\n",
    )?;
    let found = calls(&root, "caller")?;
    assert!(
        !found.iter().any(|edge| edge == "left.left exact"),
        "a parameter shadows the module import: {found:?}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_module_rebinding_shadows_an_import() -> TestResult {
    let root = support::exclusive_dir("python-module-rebinding")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(
        root.join("app.py"),
        "from left import left as call\ncall = lambda: None\ndef caller():\n    call()\n",
    )?;
    let found = calls(&root, "caller")?;
    assert!(
        !found.iter().any(|edge| edge == "left.left exact"),
        "a module rebinding shadows the import: {found:?}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_class_import_is_not_a_method_local() -> TestResult {
    let root = support::exclusive_dir("python-class-import")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(
        root.join("app.py"),
        "class Example:\n    from left import left as call\n    def caller(self):\n        call()\n",
    )?;
    let found = calls(&root, "caller")?;
    assert!(
        !found.iter().any(|edge| edge == "left.left exact"),
        "a class-body import is not visible as a bare method name: {found:?}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_nested_function_can_use_an_enclosing_import() -> TestResult {
    let root = support::exclusive_dir("python-enclosing-import")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(
        root.join("app.py"),
        "def outer():\n    from left import left as call\n    def inner():\n        call()\n    inner()\n",
    )?;
    assert_eq!(calls(&root, "inner")?, ["left.left exact"]);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_type_annotation_does_not_shadow_an_import() -> TestResult {
    let root = support::exclusive_dir("python-import-annotation")?;
    fs::write(root.join("service.py"), "class Service:\n    pass\n")?;
    fs::write(
        root.join("app.py"),
        "from service import Service\ndef caller(value: Service):\n    return Service()\n",
    )?;
    assert_eq!(calls(&root, "caller")?, ["service.Service exact"]);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_type_checking_imports_support_annotations_but_not_runtime_calls() -> TestResult {
    let root = support::exclusive_dir("python-type-checking-import")?;
    fs::write(
        root.join("service.py"),
        "class Service:\n    def run(self): pass\n\nclass Decoy:\n    def run(self): pass\n",
    )?;
    fs::write(
        root.join("other_service.py"),
        "class Service:\n    def run(self): pass\n",
    )?;
    fs::write(
        root.join("app.py"),
        "from __future__ import annotations\nimport typing\nif typing.TYPE_CHECKING:\n    from service import Service\n\ndef caller(value: Service):\n    value.run()\n\ndef runtime():\n    Service()\n",
    )?;
    assert!(
        calls(&root, "caller")?
            .iter()
            .any(|edge| edge.starts_with("service.Service.run "))
    );
    assert!(
        !calls(&root, "runtime")?
            .iter()
            .any(|edge| edge == "service.Service exact")
    );
    let local = root.join("local_case");
    fs::create_dir(&local)?;
    fs::write(local.join("typing.py"), "TYPE_CHECKING = False\n")?;
    fs::write(
        local.join("service.py"),
        "class Service:\n    def run(self): pass\n",
    )?;
    fs::write(
        local.join("other_service.py"),
        "class Service:\n    def run(self): pass\n",
    )?;
    fs::write(
        local.join("app.py"),
        "from __future__ import annotations\nimport typing\nif typing.TYPE_CHECKING:\n    from service import Service\n\ndef caller(value: Service):\n    value.run()\n",
    )?;
    assert!(
        !calls(&local, "caller")?
            .iter()
            .any(|edge| edge == "service.Service.run exact")
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_lambda_parameter_does_not_shadow_the_enclosing_import() -> TestResult {
    let root = support::exclusive_dir("python-import-lambda")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(
        root.join("app.py"),
        "from left import left as call\ndef caller():\n    other = lambda call: call()\n    call()\n\ndef lambda_only():\n    return lambda call: call()\n",
    )?;
    assert_eq!(calls(&root, "caller")?, ["left.left exact"]);
    assert!(
        !calls(&root, "lambda_only")?
            .iter()
            .any(|edge| edge == "left.left exact")
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_comprehension_binding_does_not_shadow_the_enclosing_import() -> TestResult {
    let root = support::exclusive_dir("python-import-comprehension")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(
        root.join("app.py"),
        "from left import left as call\ndef caller(items):\n    values = [call() for call in items]\n    call()\n\ndef comprehension_only(items):\n    return [call() for call in items]\n",
    )?;
    assert_eq!(calls(&root, "caller")?, ["left.left exact"]);
    assert!(
        !calls(&root, "comprehension_only")?
            .iter()
            .any(|edge| edge == "left.left exact")
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_conditional_reexports_are_not_assumed_unique() -> TestResult {
    let root = support::exclusive_dir("python-conditional-exports")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(root.join("right.py"), "def right(): pass\n")?;
    fs::write(
        root.join("facade.py"),
        "if condition:\n    from left import left as exposed\nelse:\n    from right import right as exposed\n",
    )?;
    fs::write(
        root.join("app.py"),
        "from facade import exposed\ndef caller():\n    exposed()\n",
    )?;
    fs::write(
        root.join("facade_external.py"),
        "if condition:\n    from left import left as exposed\nelse:\n    from external import unknown as exposed\n",
    )?;
    fs::write(
        root.join("facade_same.py"),
        "if condition:\n    from left import left as exposed\nelse:\n    from left import left as exposed\n",
    )?;
    fs::write(root.join("a.py"), "def alpha(): pass\n")?;
    fs::write(root.join("b.py"), "def beta(): pass\n")?;
    fs::write(
        root.join("facade_star.py"),
        "from a import *\nfrom left import *\nfrom b import *\n",
    )?;
    fs::write(
        root.join("other.py"),
        "from facade_external import exposed as external_value\nfrom facade_same import exposed as same_value\nfrom facade_star import left as star_value\ndef caller_external():\n    external_value()\n\ndef caller_same():\n    same_value()\n\ndef caller_star():\n    star_value()\n",
    )?;
    let found = calls(&root, "caller")?;
    assert!(
        !found.iter().any(|edge| edge.ends_with(" exact")),
        "a conditional re-export does not prove a unique target: {found:?}"
    );
    let external = calls(&root, "caller_external")?;
    assert!(
        !external.iter().any(|edge| edge.ends_with(" exact")),
        "an unknown alternative does not prove a unique target: {external:?}"
    );
    assert_eq!(calls(&root, "caller_same")?, ["left.left exact"]);
    assert_eq!(calls(&root, "caller_star")?, ["left.left exact"]);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_sequential_reexport_uses_the_last_binding() -> TestResult {
    let root = support::exclusive_dir("python-sequential-exports")?;
    fs::write(root.join("left.py"), "def left(): pass\n")?;
    fs::write(root.join("right.py"), "def right(): pass\n")?;
    fs::write(
        root.join("facade.py"),
        "from left import left as exposed\nfrom right import right as exposed\n",
    )?;
    fs::write(
        root.join("app.py"),
        "from facade import exposed\ndef caller():\n    exposed()\n",
    )?;
    assert_eq!(calls(&root, "caller")?, ["right.right exact"]);
    fs::write(
        root.join("facade_definition.py"),
        "def exposed(): pass\nfrom right import right as exposed\n",
    )?;
    fs::write(
        root.join("app_definition.py"),
        "from facade_definition import exposed\ndef caller_definition():\n    exposed()\n",
    )?;
    assert_eq!(calls(&root, "caller_definition")?, ["right.right exact"]);
    fs::write(
        root.join("facade_reverse.py"),
        "from right import right as exposed\ndef exposed(): pass\n",
    )?;
    fs::write(
        root.join("app_reverse.py"),
        "from facade_reverse import exposed\ndef caller_reverse():\n    exposed()\n",
    )?;
    assert_eq!(
        calls(&root, "caller_reverse")?,
        ["facade_reverse.exposed exact"]
    );
    fs::write(
        root.join("facade_uncertain.py"),
        "def conditional(): pass\nif flag:\n    from right import right as conditional\ndef reassigned(): pass\nfrom right import right as reassigned\nreassigned = lambda: None\ndef global_changed(): pass\nfrom right import right as global_changed\ndef mutate():\n    global global_changed\n    global_changed = lambda: None\nfrom right import right as conditional_definition\nif flag:\n    def conditional_definition(): pass\nfrom right import right as mutated_definition\ndef mutated_definition(): pass\nmutated_definition = lambda: None\n",
    )?;
    fs::write(
        root.join("app_uncertain.py"),
        "from facade_uncertain import conditional, reassigned, global_changed, conditional_definition, mutated_definition\ndef caller_conditional():\n    conditional()\ndef caller_reassigned():\n    reassigned()\ndef caller_global():\n    global_changed()\ndef caller_conditional_definition():\n    conditional_definition()\ndef caller_mutated_definition():\n    mutated_definition()\n",
    )?;
    for caller in [
        "caller_conditional",
        "caller_reassigned",
        "caller_global",
        "caller_conditional_definition",
        "caller_mutated_definition",
    ] {
        assert!(
            !calls(&root, caller)?
                .iter()
                .any(|edge| edge.ends_with(" exact")),
            "{caller} has an unproven binding"
        );
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn typescript_unions_reach_every_alternative() -> TestResult {
    let root = support::exclusive_dir("declared-ts")?;
    fs::write(root.join("a.ts"), TS)?;
    assert_eq!(
        calls(&root, "direct")?,
        ["a.Left.run exact", "a.Right.run exact"]
    );
    assert_eq!(calls(&root, "nullable")?, ["a.Left.run exact"]);
    assert_eq!(calls(&root, "leading")?, ["a.Right.run exact"]);
    assert_eq!(calls(&root, "both")?, ["a.Only.solo exact"]);
    assert!(
        !calls(&root, "partial")?
            .iter()
            .any(|edge| edge.ends_with("exact")),
        "an alternative without the member proves nothing"
    );
    assert!(
        !calls(&root, "elements")?
            .iter()
            .any(|edge| edge.ends_with("exact")),
        "a composed element proves nothing"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_and_php_unions_are_never_exact() -> TestResult {
    let root = support::exclusive_dir("declared-py-php")?;
    fs::write(
        root.join("a.py"),
        "class Left:\n    def run(self):\n        return 1\n\nclass Right:\n    def run(self):\n        return 2\n\ndef direct(x: Left | Right):\n    x.run()\n\ndef leading(x: None | Right):\n    x.run()\n",
    )?;
    fs::write(
        root.join("b.php"),
        "<?php\nclass PLeft { function go() { return 1; } }\nclass PRight { function go() { return 2; } }\nfunction pdirect(PLeft|PRight $x) { $x->go(); }\nfunction pleading(null|PRight $x) { $x->go(); }\n",
    )?;
    assert!(
        !calls(&root, "direct")?
            .iter()
            .any(|edge| edge.ends_with("exact")),
        "python union"
    );
    assert_eq!(calls(&root, "leading")?, ["a.Right.run exact"]);
    assert!(
        !calls(&root, "pdirect")?
            .iter()
            .any(|edge| edge.ends_with("exact")),
        "php union"
    );
    assert_eq!(calls(&root, "pleading")?, ["PRight.go exact"]);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_awaited_async_result_keeps_its_declared_receiver_type() -> TestResult {
    let root = support::exclusive_dir("python-await-result")?;
    fs::write(
        root.join("a.py"),
        "class Response:\n    def finish(self): pass\n\nclass Decoy:\n    def finish(self): pass\n\nclass Awaitable:\n    def finish(self): pass\n    def __await__(self):\n        if False: yield\n        return Response()\n\ndef sync_factory() -> Awaitable:\n    return Awaitable()\n\nasync def wrong():\n    result = await sync_factory()\n    result.finish()\n\ndef replace(fn):\n    async def replacement():\n        return Decoy()\n    return replacement\n\n@replace\nasync def decorated() -> Response:\n    return Response()\n\nasync def decorated_caller():\n    result = await decorated()\n    result.finish()\n\nclass Client:\n    async def __aenter__(self) -> Client:\n        return self\n    async def __aexit__(self, typ, value, traceback):\n        pass\n    async def send(self) -> Response:\n        return Response()\n    async def run(self):\n        result = await self.send()\n        result.finish()\n    def missing_await(self):\n        result = self.send()\n        result.finish()\n\nasync def context():\n    async with Client() as client:\n        await client.send()\n",
    )?;
    let found = calls(&root, "run")?;
    assert!(
        found
            .iter()
            .any(|edge| edge == "a.Response.finish inferred"),
        "{found:?}"
    );
    let wrong = calls(&root, "wrong")?;
    assert!(
        !wrong
            .iter()
            .any(|edge| edge.starts_with("a.Awaitable.finish ")),
        "{wrong:?}"
    );
    let decorated = calls(&root, "decorated_caller")?;
    assert!(
        !decorated
            .iter()
            .any(|edge| edge.starts_with("a.Response.finish ")),
        "{decorated:?}"
    );
    let unawaited = calls(&root, "missing_await")?;
    assert!(
        !unawaited
            .iter()
            .any(|edge| edge.starts_with("a.Response.finish ")),
        "{unawaited:?}"
    );
    let context = calls(&root, "context")?;
    assert!(
        context.iter().any(|edge| edge == "a.Client.send inferred"),
        "{context:?}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn an_own_array_type_is_no_standard_container() -> TestResult {
    let root = support::exclusive_dir("declared-array")?;
    fs::write(
        root.join("a.ts"),
        "class Array<T> { first!: Other; }\nclass Other { go() { return 1 } }\nclass Item { go() { return 2 } }\nexport function own(items: Array<Item>) { for (const x of items as any) { x.go(); } }\n",
    )?;
    assert!(
        !calls(&root, "own")?
            .iter()
            .any(|edge| edge == "a.Item.go exact"),
        "own Array"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
