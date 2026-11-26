// PreToolUse hook: blocks AI credit, Claude git identities and claude/ branches.
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const BAD = [
  /co-authored-by:.*(claude|anthropic)/i,
  /claude-session:/i,
  /generated (with|by)\b.*claude/i,
  /(assisted|created|written) (with|by)\b.*claude/i,
  /claude\.ai\/code/i,
  /noreply@anthropic\.com/i,
  /\u{1F916}/u,
];
const BAD_COMMAND = [...BAD, /\bclaude\.ai\b/i];
const WRITES_TEXT = /\bgit\s+(commit|tag|merge)\b|\bgh\s+(pr|issue|release)\b/;
const CLAUDE_BRANCH = /^(refs\/heads\/)?claude\//i;

class Block extends Error {}
const block = (reason) => { throw new Block(reason); };

// Quote-aware split into commands (arrays of words). Heredoc bodies and comments are dropped.
function tokenize(src, ps) {
  const commands = [];
  const esc = ps ? "`" : "\\";
  function parse(i, closer) {
    let words = [], word = null, skip = false, depth = 0;
    let heredocs = [];
    const add = (s) => (word = (word ?? "") + s);
    const endWord = () => {
      if (word !== null) { if (!skip) words.push(word); skip = false; word = null; }
    };
    const endCommand = () => { endWord(); skip = false; if (words.length) commands.push(words); words = []; };
    for (; i < src.length; i++) {
      const c = src[i], next = src[i + 1];
      let m;
      if (ps && c === "@" && (next === "'" || next === '"') && (m = /^@(['"])[ \t]*\r?\n/.exec(src.slice(i)))) {
        const start = i + m[0].length;
        const end = new RegExp("\\r?\\n" + m[1] + "@", "g");
        end.lastIndex = start - 1;
        const e = end.exec(src);
        add(src.slice(start, e ? e.index : src.length));
        i = e ? end.lastIndex - 1 : src.length;
      } else if (c === "'") {
        let e = src.indexOf("'", i + 1);
        if (e < 0) e = src.length;
        add(src.slice(i + 1, e));
        i = e;
      } else if (c === '"') {
        add("");
        for (i++; i < src.length && src[i] !== '"'; i++) {
          if (src[i] === esc && (ps || '"\\$`\n'.includes(src[i + 1]))) add(src[++i] ?? "");
          else if (src[i] === "$" && src[i + 1] === "(") {
            const e = parse(i + 2, ")");
            add(src.slice(i, e + 1));
            i = e;
          } else add(src[i]);
        }
      } else if (c === esc) {
        i++;
        if (src[i] === "\r") i++;
        if (src[i] !== "\n") add(src[i] ?? "");
      } else if (c === "#" && word === null) {
        while (i + 1 < src.length && src[i + 1] !== "\n") i++;
      } else if (!ps && c === "<" && next === "<" && src[i + 2] !== "<" &&
                 (m = /^<<(-?)[ \t]*(?:(['"])(.*?)\2|\\?([A-Za-z_][\w.-]*))/.exec(src.slice(i)))) {
        endWord();
        heredocs.push({ dash: m[1], end: m[3] ?? m[4] });
        i += m[0].length - 1;
      } else if (c === ">" || c === "<" || (c === "&" && next === ">")) {
        if (/^(\d|\*)$/.test(word ?? "")) word = null;
        endWord();
        while (">&<".includes(src[i + 1] || " ")) i++;
        const fd = src[i] === "&" && /^(\d|-)/.exec(src.slice(i + 1));
        if (fd) i++; else skip = true;
      } else if (c === "\n") {
        endCommand();
        for (const h of heredocs) {
          while (i + 1 < src.length) {
            let e = src.indexOf("\n", i + 1);
            if (e < 0) e = src.length;
            let line = src.slice(i + 1, e).replace(/\r$/, "");
            if (h.dash) line = line.replace(/^\t+/, "");
            if (line === h.end) { i = e; break; }
            // "EOF)" closes both the heredoc and a $( ... ).
            if (closer && line.startsWith(h.end + ")")) { i = src.indexOf(h.end, i + 1) + h.end.length - 1; break; }
            i = e;
          }
        }
        heredocs = [];
      } else if (c === " " || c === "\t" || c === "\r") {
        endWord();
      } else if (c === ")" && closer && depth === 0) {
        endCommand();
        return i;
      } else if (";&|()".includes(c) || (!ps && c === "`") || ((c === "{" || c === "}") && word === null)) {
        if (c === "(") depth++;
        if (c === ")") depth--;
        endCommand();
      } else add(c);
    }
    endCommand();
    return i;
  }
  parse(0, null);
  return commands;
}

// Splits arguments into options and positionals. `short`/`long` take a value; `attached` take an optional glued one.
function options(args, short = "", long = [], attached = "") {
  const opts = [], pos = [];
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === "--") { pos.push(...args.slice(i + 1)); break; }
    if (a.startsWith("--")) {
      const eq = a.indexOf("=");
      if (eq > 0) opts.push([a.slice(0, eq), a.slice(eq + 1)]);
      else opts.push([a, long.includes(a) ? args[++i] ?? "" : true]);
    } else if (a.startsWith("-") && a.length > 1) {
      for (let j = 1; j < a.length; j++) {
        const f = "-" + a[j];
        if (short.includes(a[j])) { opts.push([f, a.slice(j + 1) || (args[++i] ?? "")]); break; }
        opts.push([f, true]);
        if (attached.includes(a[j])) break;
      }
    } else pos.push(a);
  }
  const has = (...names) => opts.some(([n]) => names.includes(n));
  const values = (...names) => opts.filter(([n, v]) => names.includes(n) && v !== true).map(([, v]) => v);
  return { pos, has, values, value: (...names) => values(...names).pop() };
}

const nativePath = (p) =>
  process.platform === "win32" ? p.replace(/^\/([a-z])(?=\/|$)/i, "$1:") : p;
const resolveDir = (base, p) =>
  path.resolve(base, nativePath(p.replace(/^~(?=$|[\\/])/, require("os").homedir())));

function badIdentity(who, name, email) {
  if (/claude/i.test(name || "") || /^noreply@anthropic\.com$/i.test(email || "")) {
    block(`git ${who} identity is Claude (${name || ""} <${email || ""}>). Use the user's own git identity.`);
  }
}

function checkCredit(text) {
  const hit = BAD_COMMAND.find((re) => re.test(text));
  if (hit) {
    block(`this commit/PR text credits Claude or AI (${hit}). ` +
      "Remove all Co-Authored-By, Claude-Session, 'Generated with/by', claude.ai links and robot emoji, then retry.");
  }
}

function checkCommand(command, ps, startDir) {
  let dir = startDir, writesText = false;
  const exported = {};
  const files = [];
  const readable = (base, names) => names.forEach((n) => files.push(resolveDir(base, n)));

  for (let words of tokenize(command, ps)) {
    const local = {};
    let exporting = false, m;
    if ((m = /^\$env:(\w+)\s*=\s*(.*)$/is.exec(words.join(" ")))) { exported[m[1].toUpperCase()] = m[2]; continue; }
    words = words.filter((w) => w !== "}");
    while (words.length) {
      if ((m = /^([A-Za-z_]\w*)=(.*)$/s.exec(words[0]))) (exporting ? exported : local)[m[1]] = m[2];
      else if (words[0] === "export") exporting = true;
      else if (!/^(\{|!|then|do|else|elif|if|while|time|command|exec|env|sudo|nohup)$/.test(words[0])) break;
      words.shift();
    }
    if (!words.length) { Object.assign(exported, local); continue; }

    const env = { ...exported, ...local };
    const program = words[0].split(/[\\/]/).pop().toLowerCase().replace(/\.(exe|cmd)$/, "");
    const args = words.slice(1);

    if (/^(cd|chdir|sl|pushd|set-location|push-location)$/.test(program)) {
      const target = args.filter((a) => !/^(-.+|\/d)$/i.test(a)).pop();
      if (target) dir = resolveDir(dir, target);
    } else if (program === "git") {
      checkGit(args, dir, env);
    } else if (program === "gh") {
      checkGh(args, dir, env);
    }
  }

  function git(globals, cwd, env, args) {
    try {
      return execFileSync("git", [...globals, ...args], {
        cwd, env: { ...process.env, ...env }, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], timeout: 3000,
      }).trim();
    } catch { return null; }
  }

  function checkGit(args, cwd, env) {
    const globals = [], config = {};
    let gitDir = cwd, i = 0;
    for (; i < args.length && args[i].startsWith("-"); i++) {
      let a = args[i];
      if (["-C", "-c", "--git-dir", "--work-tree", "--namespace", "--config-env"].includes(a)) {
        let v = args[++i] ?? "";
        if (a === "-C") gitDir = resolveDir(gitDir, v);
        if (a === "-c") config[v.split("=")[0].toLowerCase()] = v.slice(v.indexOf("=") + 1);
        if (a === "-C" || a.endsWith("-dir") || a === "--work-tree") v = nativePath(v);
        globals.push(a, v);
      } else globals.push(a);
    }
    const sub = args[i], rest = args.slice(i + 1);
    const run = (...a) => git(globals, cwd, env, a);
    const currentBranch = () => run("symbolic-ref", "--short", "-q", "HEAD") || "";
    const noClaude = (name) => {
      if (CLAUDE_BRANCH.test(name || "")) block(`branch "${name}" uses the claude/ prefix. Pick another prefix, e.g. feat/ or chore/.`);
    };
    // Precedence: environment, then -c, then whatever git itself would use.
    const identity = (role) => {
      const upper = role.toUpperCase();
      const explicit = (part) =>
        env[`GIT_${upper}_${part.toUpperCase()}`] ?? process.env[`GIT_${upper}_${part.toUpperCase()}`] ??
        config[`${role}.${part}`] ?? config[`user.${part}`];
      let name = explicit("name"), email = explicit("email");
      if (name === undefined || email === undefined) {
        const found = /^(.*?) <(.*)> \d+ [+-]\d+$/.exec(run("var", `GIT_${upper}_IDENT`) || "");
        name ??= found?.[1];
        email ??= found?.[2];
      }
      badIdentity(role, name, email);
    };

    if (sub === "commit") {
      const o = options(rest, "mFCct", ["--message", "--file", "--author", "--date", "--reuse-message",
        "--reedit-message", "--fixup", "--squash", "--template", "--cleanup", "--trailer"], "Su");
      writesText = true;
      readable(gitDir, o.values("-F", "--file"));
      const author = o.value("--author");
      if (author !== undefined) {
        const a = /^(.*?)\s*<(.*)>/.exec(author);
        badIdentity("author", a ? a[1] : author, a?.[2]);
      } else if (o.has("--amend") && !o.has("--reset-author")) {
        const head = run("log", "-1", "--format=%an%n%ae", "HEAD");
        if (head) badIdentity("author (kept by --amend; use --reset-author)", ...head.split("\n"));
      } else identity("author");
      identity("committer");
    } else if (sub === "merge") {
      const o = options(rest, "mFsX", ["--message", "--file", "--strategy", "--strategy-option", "--into-name", "--cleanup"], "S");
      writesText = true;
      readable(gitDir, o.values("-F", "--file"));
      if (!o.has("--abort", "--quit", "--ff-only", "--squash", "--no-commit")) { identity("author"); identity("committer"); }
    } else if (sub === "tag") {
      const o = options(rest, "mFu", ["--message", "--file", "--local-user"], "n");
      writesText = true;
      readable(gitDir, o.values("-F", "--file"));
      if (o.has("-a", "-s", "-u", "-m", "-F", "--annotate", "--sign", "--local-user", "--message", "--file") &&
          !o.has("-d", "-l", "-v", "--delete", "--list", "--verify")) identity("committer");
    } else if (sub === "config") {
      const o = options(rest, "f", ["--file", "--blob", "--type", "--default", "--comment"]);
      const [key, value] = o.pos[0] === "set" ? o.pos.slice(1) : o.pos;
      const k = /^(user|author|committer)\.(name|email)$/i.exec(key || "");
      if (k && value !== undefined && !o.has("--get", "--get-all", "--get-regexp", "--unset", "--unset-all")) {
        badIdentity("config", ...(k[2].toLowerCase() === "name" ? [value] : ["", value]));
      }
    } else if (sub === "checkout" || sub === "switch") {
      const create = sub === "checkout" ? ["-b", "-B", "--orphan"] : ["-c", "-C", "--create", "--force-create", "--orphan"];
      const o = options(rest, sub === "checkout" ? "bB" : "cC", ["--create", "--force-create", "--orphan"]);
      const name = o.value(...create);
      if (name !== undefined) noClaude(name);
      // --track without a name derives the local branch from the remote one.
      else if (o.has("-t", "--track")) noClaude(o.pos[0]?.replace(/^(refs\/)?(remotes\/)?[^/]+\//, ""));
    } else if (sub === "branch") {
      const o = options(rest, "u", ["--set-upstream-to", "--contains", "--no-contains", "--merged", "--no-merged",
        "--points-at", "--sort", "--format"]);
      if (o.has("-m", "-M", "-c", "-C", "--move", "--copy")) noClaude(o.pos.at(-1));
      else if (!o.has("-d", "-D", "--delete", "-l", "--list", "-a", "--all", "-r", "--remotes", "-u", "--set-upstream-to",
        "--unset-upstream", "--edit-description", "--contains", "--no-contains", "--merged", "--no-merged",
        "--points-at", "--show-current")) noClaude(o.pos[0]);
    } else if (sub === "worktree" && rest[0] === "add") {
      noClaude(options(rest.slice(1), "bB", ["--reason"]).value("-b", "-B"));
    } else if (sub === "push") {
      const o = options(rest, "o", ["--push-option", "--repo", "--receive-pack", "--exec"]);
      if (o.has("-d", "--delete")) return;
      const refspecs = o.pos.slice(o.has("--repo") ? 0 : 1);
      if (o.has("--all", "--branches", "--mirror")) {
        noClaude(run("for-each-ref", "--format=%(refname)", "refs/heads/claude/")?.split("\n")[0]);
      } else if (!refspecs.length) {
        if (!o.has("--tags")) noClaude(currentBranch());
      }
      for (const spec of refspecs) {
        const s = spec.replace(/^\+/, "");
        if (s.startsWith(":")) continue; // deletion
        const dst = s.slice(s.lastIndexOf(":") + 1);
        noClaude(dst === "HEAD" || dst === "@" ? currentBranch() : dst);
      }
    }
  }

  function checkGh(args, cwd, env) {
    const o = options(args, "tbBHFalRTn", ["--title", "--body", "--base", "--head", "--body-file", "--notes",
      "--notes-file", "--repo", "--label", "--assignee", "--reviewer", "--milestone", "--project", "--template"]);
    const [group, action] = o.pos;
    if (!["pr", "issue", "release"].includes(group)) return;
    writesText = true;
    readable(cwd, o.values("-F", "--body-file", "--notes-file"));
    if (group === "pr" && (action === "create" || action === "new")) {
      const head = o.value("-H", "--head") ?? git([], cwd, env, ["symbolic-ref", "--short", "-q", "HEAD"]) ?? "";
      if (CLAUDE_BRANCH.test(head.replace(/^[^/:]+:/, ""))) block(`pull request head "${head}" uses the claude/ prefix. Rename the branch.`);
    }
  }

  if (!writesText && !WRITES_TEXT.test(command)) return;
  // Wrapped commands (bash -c "...") are not parsed, so find their message files by pattern.
  if (!files.length) {
    for (const m of command.matchAll(/(?:-F|--file|--body-file)\s+"?([^\s"']+)"?/g)) readable(dir, [m[1]]);
  }
  let text = command;
  for (const file of files) {
    try { if (fs.statSync(file).isFile()) text += "\n" + fs.readFileSync(file, "utf8"); } catch {}
  }
  checkCredit(text);
}

let raw = "";
process.stdin.on("data", (d) => (raw += d));
process.stdin.on("end", () => {
  try {
    const input = JSON.parse(raw);
    const tool = String(input.tool_name || "");
    const args = input.tool_input || {};
    if (tool === "Bash" || tool === "PowerShell") {
      checkCommand(String(args.command || ""), tool === "PowerShell", input.cwd || process.cwd());
    } else if (/git/i.test(tool)) {
      const hit = BAD.find((re) => re.test(JSON.stringify(args)));
      if (hit) block(`this tool call credits Claude or AI (${hit}). Remove it, then retry.`);
    }
  } catch (e) {
    // Unparseable input or an internal error never blocks.
    if (e instanceof Block) {
      console.error("Blocked: " + e.message);
      process.exitCode = 2;
    }
  }
});
