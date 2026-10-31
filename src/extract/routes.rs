//! File-based framework routes (`docs/spec.md` section 5): Next.js App Router and Pages Router,
//! SvelteKit and Astro endpoints. Each route becomes an `op` symbol named `<kind> <path>`
//! (`page /dashboard`, `GET /api/posts/[id]`, `load /article/[slug]`) on the line of its
//! handler, with a call to the handler, so `impact` of a component reaches the routes rendering
//! it. Next.js `middleware`/`proxy` files, SvelteKit hooks (`hook handle`) and `"use server"`
//! modules (`action <name>`) count too; page data exports (`generateMetadata`) are called by the
//! page route.

use crate::domain::facts::{Facts, NO_ARITY, Ref, RefKind, SymKind, Symbol};

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
const SCRIPT_EXTENSIONS: [&str; 6] = ["ts", "tsx", "js", "jsx", "mjs", "mts"];
/// Next.js App Router files that render a route segment, and metadata files (`robots.ts`).
const APP_FILES: [&str; 15] = [
    "page",
    "layout",
    "template",
    "loading",
    "error",
    "not-found",
    "global-error",
    "default",
    "robots",
    "sitemap",
    "manifest",
    "icon",
    "apple-icon",
    "opengraph-image",
    "twitter-image",
];
/// Wrappers whose first argument is the component (`export default memo(Card)`).
const WRAPPERS: [&str; 6] = [
    "memo",
    "React.memo",
    "forwardRef",
    "React.forwardRef",
    "observer",
    "withRouter",
];

/// Exports Next.js calls besides a page's component: metadata, static params, data loaders.
const DATA_EXPORTS: [&str; 6] = [
    "generateMetadata",
    "generateStaticParams",
    "generateViewport",
    "getServerSideProps",
    "getStaticProps",
    "getStaticPaths",
];
/// SvelteKit hook exports of `hooks.server`, `hooks.client` and `hooks` files.
const HOOKS: [&str; 6] = [
    "handle",
    "handleFetch",
    "handleError",
    "init",
    "reroute",
    "transport",
];

/// How a route file names its handlers.
enum Handlers {
    /// The default export (Next.js pages, layouts, Pages Router API routes).
    Default(&'static str),
    /// Exported HTTP method functions (`route.ts`, `+server.ts`, Astro endpoints).
    Methods,
    /// The given exports (`load`, `actions`) as `<export> <path>`.
    Exports(&'static [&'static str]),
    /// The file's component symbol (Svelte `+page.svelte`).
    Component(&'static str),
}

/// Adds the routes of a script or Svelte file; `component` is the index of the file's component
/// symbol for single-file components.
pub fn add(path: &str, text: &str, component: Option<u32>, facts: &mut Facts) {
    if let Some((route, handlers)) = classify(path) {
        let exports = Exports::scan(text);
        match handlers {
            Handlers::Default(kind) => {
                if exports.default_line > 0 && next_file(kind, text, exports.default.as_deref()) {
                    let handler = exports.default.as_deref();
                    let route = push(
                        facts,
                        format!("{kind} {route}"),
                        handler,
                        exports.default_line,
                    );
                    for name in DATA_EXPORTS {
                        if let Some((handler, line)) = exports.named(name, facts) {
                            call(facts, route, &handler, line);
                        }
                    }
                }
            }
            Handlers::Methods => {
                for method in METHODS {
                    if let Some((handler, line)) = exports.named(method, facts) {
                        push(facts, format!("{method} {route}"), Some(&handler), line);
                    }
                }
                // A Pages Router API route is one default handler for every method.
                if !METHODS
                    .iter()
                    .any(|method| exports.named(method, facts).is_some())
                    && exports.default_line > 0
                {
                    let handler = exports.default.as_deref();
                    push(facts, format!("api {route}"), handler, exports.default_line);
                }
            }
            Handlers::Exports(names) => {
                for name in names {
                    if let Some((handler, line)) = exports.named(name, facts) {
                        push(facts, format!("{name} {route}"), Some(&handler), line);
                    }
                }
            }
            Handlers::Component(kind) => {
                if let Some(component) = component {
                    let name = facts.symbols[component as usize].name.clone();
                    push(facts, format!("{kind} {route}"), Some(&name), 1);
                }
            }
        }
        return;
    }
    let stem = stem(path);
    if matches!(stem, "middleware" | "proxy") && is_script(path) && is_app_root_file(path) {
        let exports = Exports::scan(text);
        if let Some((handler, line)) = exports.named(stem, facts) {
            push(facts, format!("{stem} /"), Some(&handler), line);
        } else if exports.default_line > 0 {
            let handler = exports.default.as_deref();
            push(facts, format!("{stem} /"), handler, exports.default_line);
        }
        return;
    }
    if stem == "hooks" && is_script(path) && path.split('/').rev().nth(1) == Some("src") {
        for name in HOOKS {
            if let Some((handler, line)) = Exports::default().named(name, facts) {
                push(facts, format!("hook {name}"), Some(&handler), line);
            }
        }
        return;
    }
    if is_script(path) && starts_with_directive(text, "use server") {
        let actions: Vec<String> = facts
            .symbols
            .iter()
            .filter(|symbol| {
                symbol.parent.is_none() && symbol.exported && symbol.kind == Some(SymKind::Fn)
            })
            .map(|symbol| symbol.name.clone())
            .collect();
        for name in actions {
            if let Some((handler, line)) = Exports::default().named(&name, facts) {
                push(facts, format!("action {name}"), Some(&handler), line);
            }
        }
    }
}

/// Route path and handler convention of a file, by framework path convention.
fn classify(path: &str) -> Option<(String, Handlers)> {
    let segments: Vec<&str> = path.split('/').collect();
    let (file, dirs) = segments.split_last()?;
    let (stem, extension) = file.rsplit_once('.')?;
    if let Some(start) = framework_root(dirs, "routes")
        .filter(|&start| start > 0 && dirs[start - 1] == "src" && stem.starts_with('+'))
    {
        let route = route_path(&dirs[start + 1..])?;
        let handlers = match (stem, extension) {
            ("+page", "svelte") => Handlers::Component("page"),
            ("+layout", "svelte") => Handlers::Component("layout"),
            ("+error", "svelte") => Handlers::Component("error"),
            ("+server", _) if is_script(path) => Handlers::Methods,
            ("+page" | "+page.server" | "+layout" | "+layout.server", _) if is_script(path) => {
                Handlers::Exports(&["load", "actions"])
            }
            _ => return None,
        };
        return Some((route, handlers));
    }
    if !SCRIPT_EXTENSIONS.contains(&extension) {
        return None;
    }
    let pages = framework_root(dirs, "pages");
    if let Some(start) =
        framework_root(dirs, "app").filter(|&app| pages.is_none_or(|pages| app < pages))
    {
        let route = route_path(&dirs[start + 1..])?;
        if stem == "route" {
            return Some((route, Handlers::Methods));
        }
        let kind = APP_FILES.iter().find(|kind| **kind == stem)?;
        return Some((route, Handlers::Default(kind)));
    }
    let start = pages?;
    let page_name = stem.starts_with(|ch: char| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        || stem.starts_with('[');
    if !page_name {
        return None;
    }
    let mut route_dirs: Vec<&str> = dirs[start + 1..].to_vec();
    if stem != "index" {
        route_dirs.push(stem);
    }
    let route = route_path(&route_dirs)?;
    let api = route_dirs.first() == Some(&"api");
    let handlers = if api || (extension.starts_with('t') && !extension.ends_with('x')) {
        Handlers::Methods
    } else {
        Handlers::Default("page")
    };
    Some((route, handlers))
}

/// Whether an App Router file named like a generic component (`loading`, `not-found`, `icon`)
/// is a Next.js file and not one of a React app's `src/app/` folder: it imports from `next`, is a
/// client component, or its default export says what it is (`DashboardLoading`). Pages and
/// layouts always count.
fn next_file(kind: &str, text: &str, handler: Option<&str>) -> bool {
    if matches!(kind, "page" | "layout") {
        return true;
    }
    let suffix: String = kind
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect();
    text.contains("from \"next")
        || text.contains("from 'next")
        || starts_with_directive(text, "use client")
        || handler.is_some_and(|handler| handler.ends_with(&suffix))
}

/// Index of the framework directory: the first `name` segment at the top or below `src`, else
/// the first one (monorepo `apps/web/app`).
fn framework_root(dirs: &[&str], name: &str) -> Option<usize> {
    let candidates: Vec<usize> = (0..dirs.len()).filter(|&i| dirs[i] == name).collect();
    candidates
        .iter()
        .copied()
        .find(|&i| i == 0 || dirs[i - 1] == "src")
        .or_else(|| candidates.first().copied())
}

/// URL path of route directories: route groups `(name)` and parallel slots `@name` vanish,
/// interception markers `(.)` are dropped, private `_folders` are no routes.
fn route_path(dirs: &[&str]) -> Option<String> {
    let mut route = String::new();
    for dir in dirs {
        if dir.starts_with('_') {
            return None;
        }
        let dir = dir.trim_start_matches(['(', '.', ')']);
        let group = dir.ends_with(')') || dir.starts_with('@') && !dir.starts_with("@[");
        if dir.is_empty() || group {
            continue;
        }
        route.push('/');
        route.push_str(dir);
    }
    if route.is_empty() {
        route.push('/');
    }
    Some(route)
}

/// Whether a file sits where Next.js looks for `middleware.ts`: the project root or `src/`,
/// also of a monorepo app (`apps/web/src/middleware.ts`).
fn is_app_root_file(path: &str) -> bool {
    let mut dirs: Vec<&str> = path.split('/').collect();
    dirs.pop();
    if dirs.last() == Some(&"src") {
        dirs.pop();
    }
    dirs.is_empty() || (dirs.len() == 2 && dirs[0] == "apps")
}

fn stem(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.split('.').next().unwrap_or(file)
}

fn is_script(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, extension)| SCRIPT_EXTENSIONS.contains(&extension))
}

/// Whether the first statement is the directive (`"use server"`), after comments.
fn starts_with_directive(text: &str, directive: &str) -> bool {
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("//") {
            rest = after.split_once('\n').map_or("", |(_, tail)| tail);
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, tail)| tail);
        } else {
            break;
        }
    }
    [format!("\"{directive}\""), format!("'{directive}'")]
        .iter()
        .any(|quoted| rest.starts_with(quoted.as_str()))
}

/// Exports found by scanning the source: the default export's line (0: none) and local name
/// (none when anonymous), and `export { a as b }` renames.
#[derive(Default)]
struct Exports {
    default: Option<String>,
    default_line: u32,
    renamed: Vec<(String, String, u32)>,
}

impl Exports {
    fn scan(text: &str) -> Self {
        let mut exports = Self::default();
        let mut from = 0;
        while let Some(offset) = text[from..].find("export") {
            let start = from + offset;
            from = start + "export".len();
            let boundary = start == 0
                || !text[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$' || ch == '.');
            if !boundary {
                continue;
            }
            let line = text[..start].matches('\n').count() as u32 + 1;
            let rest = text[from..].trim_start();
            if let Some(rest) = rest.strip_prefix("default") {
                if exports.default_line == 0 {
                    exports.default = default_name(rest);
                    exports.default_line = line;
                }
            } else if let Some(rest) = rest.strip_prefix('{') {
                let list = rest.split('}').next().unwrap_or_default();
                for item in list.split(',') {
                    let mut words = item.split_whitespace();
                    let (Some(local), Some("as"), Some(exported)) =
                        (words.next(), words.next(), words.next())
                    else {
                        continue;
                    };
                    if exported == "default" && exports.default_line == 0 {
                        exports.default = Some(local.to_owned());
                        exports.default_line = line;
                    } else {
                        exports
                            .renamed
                            .push((local.to_owned(), exported.to_owned(), line));
                    }
                }
            }
        }
        exports
    }

    /// Handler and line of an export: a top-level exported symbol of that name, else a local
    /// exported under that name.
    fn named(&self, name: &str, facts: &Facts) -> Option<(String, u32)> {
        if let Some(symbol) = facts
            .symbols
            .iter()
            .find(|symbol| symbol.parent.is_none() && symbol.exported && symbol.name == name)
        {
            return Some((symbol.name.clone(), symbol.line));
        }
        self.renamed
            .iter()
            .find(|(_, exported, _)| exported == name)
            .map(|(local, _, line)| (local.clone(), *line))
    }
}

/// Local name of `export default <...>`: a named function or class, an identifier, or the
/// first argument of a component wrapper. Anonymous defaults have none.
fn default_name(rest: &str) -> Option<String> {
    let mut rest = rest.trim_start();
    rest = rest.strip_prefix("async ").map_or(rest, str::trim_start);
    for keyword in ["function", "class"] {
        if let Some(after) = rest.strip_prefix(keyword) {
            let after = after.trim_start_matches('*').trim_start();
            return identifier(after).map(str::to_owned);
        }
    }
    let name = rest
        .split(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '$' | '.')))
        .next()
        .filter(|name| !name.is_empty())?;
    let after = rest[name.len()..].trim_start();
    if WRAPPERS.contains(&name) {
        let argument = after.strip_prefix('(')?.trim_start();
        return identifier(argument).map(str::to_owned);
    }
    let ends = after.is_empty() || after.starts_with([';', '\n', '\r']);
    (ends && !name.contains('.') && identifier(name).is_some()).then(|| name.to_owned())
}

fn identifier(text: &str) -> Option<&str> {
    let end = text
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$'))
        .unwrap_or(text.len());
    let name = &text[..end];
    name.starts_with(|ch: char| ch.is_alphabetic() || ch == '_' || ch == '$')
        .then_some(name)
}

/// Pushes the route symbol on the handler's lines with a call to the handler and returns its
/// index; an anonymous handler (`export default NextAuth(options)`) leaves the route on the
/// export line.
fn push(facts: &mut Facts, name: String, handler: Option<&str>, line: u32) -> u32 {
    let (line, end_line) = facts
        .symbols
        .iter()
        .find(|symbol| symbol.parent.is_none() && Some(symbol.name.as_str()) == handler)
        .map_or((line, line), |symbol| (symbol.line, symbol.end_line));
    let route = facts.symbols.len() as u32;
    facts.symbols.push(Symbol {
        kind: Some(SymKind::Op),
        name,
        line,
        end_line,
        exported: true,
        ..Symbol::default()
    });
    if let Some(handler) = handler {
        call(facts, route, handler, line);
    }
    route
}

fn call(facts: &mut Facts, route: u32, handler: &str, line: u32) {
    facts.refs.push(Ref {
        kind: RefKind::Call,
        name: handler.to_owned(),
        qualifier: None,
        scope: Some(route),
        line,
        arity: NO_ARITY,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routes(path: &str) -> Option<String> {
        classify(path).map(|(route, handlers)| {
            let kind = match handlers {
                Handlers::Default(kind) | Handlers::Component(kind) => kind,
                Handlers::Methods => "methods",
                Handlers::Exports(_) => "exports",
            };
            format!("{kind} {route}")
        })
    }

    #[test]
    fn route_paths_follow_framework_conventions() {
        let cases = [
            ("app/page.tsx", Some("page /")),
            (
                "app/(dashboard)/dashboard/billing/page.tsx",
                Some("page /dashboard/billing"),
            ),
            (
                "src/app/api/posts/[postId]/route.ts",
                Some("methods /api/posts/[postId]"),
            ),
            (
                "apps/web/app/@modal/(.)photo/[id]/page.tsx",
                Some("page /photo/[id]"),
            ),
            ("app/_components/page.tsx", None),
            ("app/components/button.tsx", None),
            ("pages/index.tsx", Some("page /")),
            ("src/pages/blog/[slug].tsx", Some("page /blog/[slug]")),
            (
                "pages/api/auth/[...nextauth].ts",
                Some("methods /api/auth/[...nextauth]"),
            ),
            ("src/pages/Home.tsx", None),
            ("pages/_app.tsx", None),
            ("src/pages/app/users.tsx", Some("page /app/users")),
            ("src/routes/+page.svelte", Some("page /")),
            (
                "src/routes/profile/@[user]/+layout.svelte",
                Some("layout /profile/@[user]"),
            ),
            (
                "src/routes/(app)/login/+page.server.js",
                Some("exports /login"),
            ),
            (
                "src/routes/api/items/+server.ts",
                Some("methods /api/items"),
            ),
            ("src/routes/Nav.svelte", None),
        ];
        for (path, want) in cases {
            assert_eq!(routes(path).as_deref(), want, "{path}");
        }
    }

    #[test]
    fn generic_app_names_need_a_next_signal() {
        assert!(next_file("page", "", None));
        assert!(next_file("loading", "", Some("DashboardLoading")));
        assert!(next_file(
            "not-found",
            "import Link from \"next/link\"",
            Some("Missing")
        ));
        assert!(!next_file(
            "not-found",
            "import { Link } from '@/ui'",
            Some("NotFoundRoute")
        ));
    }

    #[test]
    fn middleware_only_at_the_app_root() {
        assert!(is_app_root_file("middleware.ts"));
        assert!(is_app_root_file("src/middleware.ts"));
        assert!(is_app_root_file("apps/web/src/middleware.ts"));
        assert!(!is_app_root_file("src/server/middleware.ts"));
    }

    #[test]
    fn default_exports_name_their_handler() {
        let cases = [
            (
                "export default async function DashboardPage() {}",
                Some("DashboardPage"),
            ),
            (
                "export default class Shell extends Component {}",
                Some("Shell"),
            ),
            (
                "const Page = () => null\nexport default Page;\n",
                Some("Page"),
            ),
            ("export default memo(Card)", Some("Card")),
            ("export { handler as default }", Some("handler")),
            ("export default () => null", None),
            ("export default defineConfig({})", None),
        ];
        for (text, want) in cases {
            assert_eq!(Exports::scan(text).default.as_deref(), want, "{text}");
        }
    }

    #[test]
    fn use_server_directive_after_comments() {
        assert!(starts_with_directive(
            "// actions\n'use server'\n",
            "use server"
        ));
        assert!(!starts_with_directive(
            "const x = 'use server'",
            "use server"
        ));
    }
}
