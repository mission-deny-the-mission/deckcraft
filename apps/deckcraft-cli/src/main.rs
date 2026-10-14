//! Headless DeckCraft.
//!
//! ```text
//! deckcraft-cli info FILE                       # slides, titles, layouts (JSON)
//! deckcraft-cli render (FILE | --sample) [--slide N | --all] [--scale S] OUT.png|OUT_DIR
//! deckcraft-cli render-transition FILE --slide N --t T[,T…] [--width PX] OUT.png   # transition frame(s)
//! deckcraft-cli convert IN OUT                  # .deckcraft ⇄ .pptx, outline .txt, .png (first slide)
//! deckcraft-cli run [--in FILE | --sample] [--cmd ID[=JSON]]... [--save OUT] [--export OUT] [--print]
//! deckcraft-cli commands [FILTER]               # list commands (JSON)
//! deckcraft-cli describe ID                     # one command
//! deckcraft-cli app [--port PORT] COMMAND [JSON] | --method METHOD [JSON]   # drive the running app
//! deckcraft-cli mcp [--connect PORT] [--sample] # MCP server over stdio
//! deckcraft-cli store-write PATH FILE [--store local --store-root DIR | --store webdav --store-url URL [--store-user U --store-password P | --store-token T]] [--etag TAG] [--atomic]
//! deckcraft-cli store-read PATH OUT | store-stat PATH | store-list [PREFIX] | store-remove PATH | store-probe | store-recover  # same --store flags
//! deckcraft-cli links | --version
//! ```
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::process::ExitCode;

use deckcraft_engine::Session;
use serde_json::{Value, json};

macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(e) = writeln!(std::io::stdout(), $($arg)*) {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
            eprintln!("deckcraft-cli: can't write to stdout: {e}");
            std::process::exit(1);
        }
    }};
}

const USAGE: &str = "usage: deckcraft-cli info FILE
       deckcraft-cli render (FILE | --sample) [--slide N | --all] [--scale S] [--edit] OUT
       deckcraft-cli render-transition FILE --slide N --t T[,T...] [--width PX] OUT.png
       deckcraft-cli convert IN OUT
       deckcraft-cli run [--in FILE | --sample] [--cmd ID[=JSON]]... [--save OUT] [--export OUT] [--print]
       deckcraft-cli commands [FILTER]
       deckcraft-cli describe COMMAND
       deckcraft-cli app [--port PORT] COMMAND [JSON] | --method METHOD [JSON]
       deckcraft-cli mcp [--connect PORT] [--sample]
       deckcraft-cli store-write PATH FILE [--store local --store-root DIR | --store webdav
                 --store-url URL [--store-user U --store-password P | --store-token T]] [--etag TAG] [--atomic]
                                           upload local FILE to storage at PATH (--etag writes
                                           only if the stored version still carries TAG)
       deckcraft-cli store-read PATH OUT [--store …]  download storage PATH to local file OUT
       deckcraft-cli store-stat PATH [--store …]   metadata (size, ETag) as JSON
       deckcraft-cli store-list [PREFIX] [--store …] direct children as JSON
       deckcraft-cli store-remove PATH [--store …] delete (missing: success)
       deckcraft-cli store-probe [--store …]       capabilities + server probe as JSON
       deckcraft-cli store-recover [--store …]     finish or drop interrupted writes, as JSON
                 --store-root is a local/NAS/synced folder; --store-url is the WebDAV
                 collection, e.g. http://ocis.lan:9200/remote.php/dav/files/alice/Decks/
                 (plain http only: LAN, VPN or SSH tunnel; https is refused).
                 WebDAV auth is Basic (--store-user/--store-password) or an OIDC
                 Bearer token (--store-token, or $CRAFT_DAV_TOKEN for k8s secretKeyRef),
                 never both.
       deckcraft-cli links | --version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        Some("--version" | "-V" | "version") => {
            outln!("deckcraft-cli {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("info") => report(info(rest)),
        Some("render") => report(render(rest)),
        Some("render-transition") => report(render_transition(rest)),
        Some("convert") => report(convert(rest)),
        Some("run") => report(run(rest)),
        Some("commands") => {
            let s = Session::new();
            let f = rest.first().map(|x| x.to_lowercase());
            let list: Vec<Value> = s
                .commands()
                .into_iter()
                .filter(|c| f.as_ref().is_none_or(|f| c.id.to_lowercase().contains(f.as_str()) || c.label.to_lowercase().contains(f.as_str())))
                .map(|c| serde_json::to_value(c).unwrap_or_default())
                .collect();
            outln!("{}", serde_json::to_string_pretty(&list).unwrap_or_default());
            ExitCode::SUCCESS
        }
        Some("describe") => report(describe(rest.first().map(String::as_str))),
        Some("app") => report(app(rest)),
        Some("mcp") => report(mcp(rest)),
        Some(c) if c.starts_with("store-") => report(store_command(c, rest)),
        Some("links") => {
            use deckcraft_engine::links::*;
            outln!("Discord   {DISCORD}\nWebsite   {WEBSITE}\nApp page  {APP_PAGE}\nGitHub    {GITHUB}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            eprintln!("\nCommunity: {}  ·  {}", deckcraft_engine::links::DISCORD, deckcraft_engine::links::APP_PAGE);
            ExitCode::FAILURE
        }
    }
}

fn report(r: Result<(), String>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("deckcraft-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open(s: &mut Session, input: Option<&str>, sample: bool) -> Result<(), String> {
    if sample {
        return deckcraft_engine::sample::open_sample(s).map_err(|e| e.to_string());
    }
    match input {
        Some(p) => s.execute("file.open", &json!({"path": p})).map(|_| ()).map_err(|e| e.to_string()),
        None => s.execute("file.new", &json!({})).map(|_| ()).map_err(|e| e.to_string()),
    }
}

fn info(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("info needs a FILE")?;
    let mut s = Session::new();
    open(&mut s, Some(path), false)?;
    let v = s.execute("document.inspect", &json!({})).map_err(|e| e.to_string())?;
    outln!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}

fn render(args: &[String]) -> Result<(), String> {
    let mut input = None;
    let mut sample = false;
    let mut slide: Option<usize> = None;
    let mut all = false;
    let mut scale = 2.0;
    let mut edit = false;
    let mut out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sample" => sample = true,
            "--slide" => slide = Some(it.next().and_then(|v| v.parse().ok()).ok_or("--slide needs a number")?),
            "--all" => all = true,
            "--edit" => edit = true,
            "--scale" => scale = it.next().and_then(|v| v.parse().ok()).ok_or("--scale needs a number")?,
            other
                if input.is_none()
                    && !sample
                    && !other.starts_with("--")
                    && out.is_none()
                    && !(other.ends_with(".png") || other.ends_with(".jpg"))
                    || (input.is_none() && !sample && std::path::Path::new(other).is_file()) =>
            {
                input = Some(other.to_string())
            }
            other => out = Some(other.to_string()),
        }
    }
    let out = out.ok_or("render needs an output path")?;
    let mut s = Session::new();
    open(&mut s, input.as_deref(), sample)?;
    let st = s.doc().map_err(|e| e.to_string())?;
    let n = st.doc.slides.len();
    let list: Vec<usize> = if all { (0..n).collect() } else { vec![slide.unwrap_or(0)] };
    if all {
        std::fs::create_dir_all(&out).map_err(|e| format!("{out}: {e}"))?;
    }
    for i in list {
        if i >= n {
            return Err(format!("no slide {i} (the deck has {n})"));
        }
        let (png, w, h) = deckcraft_engine::cmd::file::render_png(&st.doc, i, scale, edit);
        let path = if all { format!("{out}/slide-{:02}.png", i + 1) } else { out.clone() };
        std::fs::write(&path, png).map_err(|e| format!("{path}: {e}"))?;
        outln!("{path} ({w}×{h})");
    }
    Ok(())
}

/// Frames of slide N's transition (0-based N; from slide N-1, or black for the first slide) at
/// the given progress values. With several values, OUT gets `-t<value>` before its extension.
fn render_transition(args: &[String]) -> Result<(), String> {
    use deckcraft_engine::render;
    let mut input = None;
    let mut slide: Option<usize> = None;
    let mut ts: Vec<f64> = Vec::new();
    let mut width: Option<f64> = None;
    let mut out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--slide" => slide = Some(it.next().and_then(|v| v.parse().ok()).ok_or("--slide needs a number")?),
            "--t" => {
                let v = it.next().ok_or("--t needs a value")?;
                for x in v.split(',') {
                    let t: f64 = x.trim().parse().map_err(|_| format!("bad --t value `{x}`"))?;
                    if !t.is_finite() {
                        return Err(format!("bad --t value `{x}`"));
                    }
                    ts.push(t);
                }
            }
            "--width" => {
                width = Some(it.next().and_then(|v| v.parse().ok()).filter(|w: &f64| w.is_finite() && *w >= 1.0).ok_or("--width needs pixels")?)
            }
            other if input.is_none() => input = Some(other.to_string()),
            other => out = Some(other.to_string()),
        }
    }
    let input = input.ok_or("render-transition needs a FILE")?;
    let out = out.ok_or("render-transition needs an output path")?;
    let i = slide.ok_or("render-transition needs --slide N")?;
    if ts.is_empty() {
        return Err("render-transition needs --t".into());
    }
    let mut s = Session::new();
    open(&mut s, Some(&input), false)?;
    let st = s.doc().map_err(|e| e.to_string())?;
    let doc = &st.doc;
    let n = doc.slides.len();
    let Some(sl) = doc.slides.get(i) else { return Err(format!("no slide {i} (the deck has {n})")) };
    let sw = doc.slide_size.width.max(1.0);
    let w = width.unwrap_or(sw * 2.0).min(render::MAX_SIDE as f64);
    let opts = render::RenderOpts { scale: w / sw, ..Default::default() };
    let tr = sl.transition.clone().unwrap_or_default();
    // Render both slides once and composite every frame from them (morph re-renders per frame).
    let prev = i.checked_sub(1);
    let cached = (tr.kind != "morph").then(|| {
        let parts = render::uses_slide_parts(&tr.kind, &tr.option).then(|| render::SlideParts::render(doc, prev, i, &opts));
        (prev.map(|p| render::render_slide(doc, p, &opts)), render::render_slide(doc, i, &opts), parts)
    });
    let aspect = sw / doc.slide_size.height.max(1.0);
    for t in &ts {
        let img = match &cached {
            Some((old, new, parts)) => {
                render::composite_transition(old.as_ref(), new, parts.as_ref(), &render::transition_frame(&tr.kind, &tr.option, *t, aspect))
            }
            None => render::render_transition(doc, i, *t, &opts),
        };
        let path = if ts.len() == 1 {
            out.clone()
        } else {
            let p = std::path::Path::new(&out);
            let stem = p.file_stem().and_then(|x| x.to_str()).unwrap_or("frame");
            let name = format!("{stem}-t{t:.3}.png");
            p.with_file_name(name).to_string_lossy().into_owned()
        };
        std::fs::write(&path, img.to_png()).map_err(|e| format!("{path}: {e}"))?;
        outln!("{path} ({}×{}) {} {} t={t}", img.width, img.height, tr.kind, tr.option);
    }
    Ok(())
}

fn convert(args: &[String]) -> Result<(), String> {
    let (Some(input), Some(out)) = (args.first(), args.get(1)) else { return Err("convert needs IN and OUT".into()) };
    let mut s = Session::new();
    open(&mut s, Some(input), false)?;
    let fmt = deckcraft_engine::cmd::file::format_for_path(out);
    let r = if matches!(fmt, "png" | "jpeg") {
        s.execute("file.export", &json!({"path": out, "slide": 0}))
    } else {
        s.execute("file.saveAs", &json!({"path": out}))
    };
    r.map_err(|e| e.to_string())?;
    outln!("{out}");
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let mut input = None;
    let mut sample = false;
    let mut cmds = vec![];
    let mut save = None;
    let mut export = None;
    let mut print = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--in" => input = Some(it.next().cloned().ok_or("--in needs a file")?),
            "--sample" => sample = true,
            "--cmd" => cmds.push(it.next().cloned().ok_or("--cmd needs ID[=JSON]")?),
            "--save" => save = Some(it.next().cloned().ok_or("--save needs a path")?),
            "--export" => export = Some(it.next().cloned().ok_or("--export needs a path")?),
            "--print" => print = true,
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    let mut s = Session::new();
    open(&mut s, input.as_deref(), sample)?;
    for c in cmds {
        let (id, p) = match c.split_once('=') {
            Some((id, j)) => (id.to_string(), serde_json::from_str::<Value>(j).map_err(|e| format!("{id}: bad JSON: {e}"))?),
            None => (c.clone(), json!({})),
        };
        let r = s.execute(&id, &p).map_err(|e| e.to_string())?;
        if print {
            outln!("{id} → {r}");
        }
    }
    if let Some(p) = save {
        s.execute("file.saveAs", &json!({"path": p})).map_err(|e| e.to_string())?;
    }
    if let Some(p) = export {
        s.execute("file.export", &json!({"path": p})).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn describe(id: Option<&str>) -> Result<(), String> {
    let id = id.ok_or("describe needs a command id")?;
    let c = deckcraft_engine::find_command(id).ok_or_else(|| format!("unknown command `{id}`"))?;
    outln!("{}  —  {}\n  where:    {}\n  shortcut: {}\n  params:   {}", c.id, c.label, c.menu.join(" ▸ "), c.shortcut.unwrap_or("—"), c.params);
    Ok(())
}

fn app(args: &[String]) -> Result<(), String> {
    use deckcraft_mcp::{Backend, Remote, control_addr};
    let mut port = deckcraft_mcp::DEFAULT_PORT.to_string();
    let mut method: Option<String> = None;
    let mut rest = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => port = it.next().cloned().ok_or("--port needs a value")?,
            "--method" => method = Some(it.next().cloned().ok_or("--method needs a name")?),
            other => rest.push(other.to_string()),
        }
    }
    let mut r = Remote::connect(&control_addr(&port))
        .map_err(|e| format!("no DeckCraft app on port {port} ({e}); start it with `deckcraft --control {port}`"))?;
    let parse = |s: Option<&String>| -> Result<Value, String> {
        s.map(|j| serde_json::from_str(j).map_err(|e| format!("bad JSON: {e}"))).transpose().map(|v| v.unwrap_or(json!({})))
    };
    let v = match method {
        Some(m) => r.call(&m, parse(rest.first())?)?,
        None => {
            let id = rest.first().ok_or("app needs a COMMAND or --method")?;
            r.call("engine.execute", json!({"command": id, "params": parse(rest.get(1))?}))?
        }
    };
    outln!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}

fn mcp(args: &[String]) -> Result<(), String> {
    use deckcraft_mcp::{Backend, Headless, Remote, Server, control_addr};
    let mut connect: Option<String> = None;
    let mut sample = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--connect" => connect = Some(it.next().cloned().ok_or("--connect needs a port or host:port")?),
            "--sample" => sample = true,
            other => return Err(format!("unknown mcp option `{other}`")),
        }
    }
    let backend: Box<dyn Backend> = match connect {
        Some(c) => Box::new(Remote::connect(&control_addr(&c)).map_err(|e| format!("can't reach the app at {c}: {e}"))?),
        None => {
            let mut h = Headless::new();
            if sample {
                deckcraft_engine::sample::open_sample(&mut h.session).map_err(|e| e.to_string())?;
            }
            Box::new(h)
        }
    };
    let mut server = Server::new(backend);
    let stdin = std::io::stdin();
    server.serve(stdin.lock(), std::io::stdout()).map_err(|e| e.to_string())
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Open the storage backend named by the `--store*` flags: `local` (a folder at `--store-root`)
/// or `webdav` (a collection at `--store-url`, Basic auth via `--store-user`/`--store-password`
/// or an OIDC/Bearer token via `--store-token`; the two schemes are mutually exclusive).
/// The token may also arrive via `$CRAFT_DAV_TOKEN` (k8s secretKeyRef) instead of argv.
fn open_store(command: &str, args: &[String]) -> Result<Box<dyn deckcraft_storage::StorageBackend>, String> {
    match arg_value(args, "--store").as_deref().unwrap_or("local") {
        "local" => {
            let root = arg_value(args, "--store-root").ok_or_else(|| format!("{command}: --store local needs --store-root DIR"))?;
            if arg_value(args, "--store-url").is_some() {
                return Err(format!("{command}: --store-url is for --store webdav, not local"));
            }
            deckcraft_storage::local::LocalBackend::open(std::path::Path::new(&root))
                .map(|b| Box::new(b) as Box<dyn deckcraft_storage::StorageBackend>)
                .map_err(|e| format!("{command}: {e}"))
        }
        "webdav" => {
            let url = arg_value(args, "--store-url").ok_or_else(|| format!("{command}: --store webdav needs --store-url URL"))?;
            if arg_value(args, "--store-root").is_some() {
                return Err(format!("{command}: --store-root is for --store local, not webdav"));
            }
            let (user, password, token) =
                (arg_value(args, "--store-user"), arg_value(args, "--store-password"), arg_value(args, "--store-token"));
            let token = token.or_else(|| std::env::var("CRAFT_DAV_TOKEN").ok().filter(|t| !t.is_empty()));
            let backend = if let Some(t) = token {
                if user.is_some() || password.is_some() {
                    return Err(format!("{command}: --store-token cannot be combined with --store-user/--store-password (pick one auth scheme)"));
                }
                deckcraft_storage::webdav::WebDavBackend::with_bearer(&url, &t)
            } else {
                deckcraft_storage::webdav::WebDavBackend::new(&url, user.as_deref(), password.as_deref())
            };
            backend
                .map(|b| Box::new(b) as Box<dyn deckcraft_storage::StorageBackend>)
                .map_err(|e| format!("{command}: {e}"))
        }
        other => Err(format!("{command}: unknown --store `{other}` (local or webdav)")),
    }
}

/// The positional arguments of a `store-*` command: option values (`--store local`, …) are not
/// positionals, wherever they appear.
fn store_positionals(args: &[String]) -> Vec<String> {
    const TAKES_VALUE: &[&str] =
        &["--store", "--store-root", "--store-url", "--store-user", "--store-password", "--store-token", "--etag"];
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg.starts_with("--") {
            i += if TAKES_VALUE.contains(&arg.as_str()) { 2 } else { 1 };
        } else {
            out.push(arg.clone());
            i += 1;
        }
    }
    out
}

fn store_command(command: &str, rest: &[String]) -> Result<(), String> {
    let positional = store_positionals(rest);
    let pos = |i: usize| positional.get(i).cloned().ok_or_else(|| format!("{command}: missing argument\n\n{USAGE}"));
    let store = open_store(command, rest)?;
    match command {
        "store-write" => {
            let (path, file) = (pos(0)?, pos(1)?);
            let bytes = std::fs::read(&file).map_err(|e| format!("{file}: {e}"))?;
            let opts =
                deckcraft_storage::WriteOptions { if_match: arg_value(rest, "--etag"), atomic: rest.iter().any(|a| a == "--atomic") };
            let meta = store.write(&path, &bytes, &opts).map_err(|e| format!("{path}: {e}"))?;
            let summary = serde_json::to_string_pretty(&json!({"path": path, "size": meta.size, "etag": meta.etag})).unwrap_or_default();
            outln!("{summary}");
            eprintln!("wrote {path} ({} bytes)", bytes.len());
            Ok(())
        }
        "store-read" => {
            let (path, out) = (pos(0)?, pos(1)?);
            let bytes = store.read(&path).map_err(|e| format!("{path}: {e}"))?;
            std::fs::write(&out, &bytes).map_err(|e| format!("{out}: {e}"))?;
            eprintln!("wrote {out} ({} bytes)", bytes.len());
            Ok(())
        }
        "store-stat" => {
            let path = pos(0)?;
            let meta = store.stat(&path).map_err(|e| format!("{path}: {e}"))?;
            let summary = serde_json::to_string_pretty(
                &json!({"path": path, "size": meta.size, "etag": meta.etag, "modified": meta.modified, "isDir": meta.is_dir}),
            )
            .unwrap_or_default();
            outln!("{summary}");
            Ok(())
        }
        "store-list" => {
            let prefix = positional.first().cloned().unwrap_or_default();
            let entries = store
                .list(&prefix)
                .map_err(|e| format!("{}: {e}", if prefix.is_empty() { "(root)" } else { &prefix }))?;
            let list: Vec<Value> =
                entries.iter().map(|e| json!({"path": e.path, "size": e.meta.size, "etag": e.meta.etag, "isDir": e.meta.is_dir})).collect();
            let summary = serde_json::to_string_pretty(&list).unwrap_or_default();
            outln!("{summary}");
            Ok(())
        }
        "store-remove" => {
            let path = pos(0)?;
            store.remove(&path).map_err(|e| format!("{path}: {e}"))?;
            eprintln!("removed {path}");
            Ok(())
        }
        "store-probe" => {
            let caps = store.capabilities();
            let probe = store.probe().map_err(|e| e.to_string())?;
            let summary = serde_json::to_string_pretty(&json!({
                "atomicWrite": caps.atomic_write, "conditionalWrite": caps.conditional_write, "list": caps.list,
                "etag": caps.etag, "persistent": caps.persistent,
                "davClasses": probe.dav_classes, "notes": probe.notes,
            }))
            .unwrap_or_default();
            outln!("{summary}");
            Ok(())
        }
        "store-recover" => {
            let report = store.recover().map_err(|e| e.to_string())?;
            let summary = serde_json::to_string_pretty(
                &json!({"completed": report.completed, "dropped": report.dropped, "removedTemp": report.removed_temp}),
            )
            .unwrap_or_default();
            outln!("{summary}");
            Ok(())
        }
        _ => Err(format!("unknown store command `{command}`\n\n{USAGE}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn store_flags_open_the_right_backend_or_explain() {
        assert!(open_store("store-write", &args(&["--store", "local"])).is_err());
        assert!(open_store("store-write", &args(&["--store", "webdav"])).is_err());
        assert!(open_store("store-write", &args(&["--store", "tape", "--store-root", "x"])).is_err());
        assert!(open_store("store-write", &args(&["--store", "local", "--store-root", "x", "--store-url", "http://h/d/"])).is_err());
        assert!(
            open_store(
                "store-write",
                &args(&["--store", "webdav", "--store-url", "http://h/d/", "--store-token", "t", "--store-user", "u"])
            )
            .is_err()
        );
        assert!(open_store("store-write", &args(&["--store", "webdav", "--store-url", "https://h/d/"])).is_err());
        let dir = std::env::temp_dir().join(format!("deckcraft-cli-store-test-{}", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        assert!(open_store("store-list", &args(&["--store", "local", "--store-root", &root])).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_positionals_skip_option_values_anywhere() {
        let got = store_positionals(&args(&["--store", "local", "a/b.pptx", "--store-root", "x", "--atomic"]));
        assert_eq!(got, args(&["a/b.pptx"]));
    }
}
