#!/usr/bin/env python3
"""Synthetic labelled terminal screens for the row classifier.

    generate.py [--screens 6000] [--seed 20261112] > train.txt

Writes the corpus format documented in data/heldout.txt: shell sessions
(bash, zsh, fish, starship, powerlevel10k, two-line, (venv), root,
PowerShell prompts) with their output, Claude Code sessions (tool calls,
`⎿` output, plans, answers with code blocks, input and permission boxes),
rendered docs and box-drawn TUIs, at widths from 40 to 200 columns with
terminal soft wraps and application hard wraps.

Deterministic for a seed (no set iteration, no hash()). Plain Python, no
dependencies. Everything is fictional: example.invalid hosts, documentation
IP ranges, /home/user paths, made-up project and tool names. It never reads
data/heldout.txt.
"""

import argparse
import random
import sys
import textwrap

R = random.Random(0)


class Cmd(str):
    """An output row that shows a command (make's echoed recipe lines, a
    Dockerfile's RUN steps, a script or workflow printed by cat): labelled
    K, code, since people want to copy and run those. Slicing or joining
    drops the mark, so rows inside an agent's tool output stay output."""


def pick(seq):
    return seq[R.randrange(len(seq))]


def chance(p):
    return R.random() < p


# ---------------------------------------------------------------- vocabulary

USERS = ["user", "dev", "ops", "alex", "sam", "kim", "ana", "lee", "jo", "deploy", "ci", "max", "noor", "ivy"]
HOSTS = ["devbox", "laptop", "web-01", "web-02", "build-3", "ci-runner", "mbp", "pi", "vm-2", "api-02", "db-1",
         "workstation", "edge-7", "staging", "lab", "nas"]
PROJECTS = ["webapp", "api", "shop", "tiny-http", "reports", "svc", "notes", "infra", "cli", "game", "docs",
            "pipeline", "limiter", "billing", "auth", "gateway", "site", "worker", "mobile", "sdk", "dash"]
BRANCHES = ["main", "dev", "feature/login-form", "fix/flaky-test", "release/2026.11", "chore/deps", "feat/quota",
            "wip", "feature/rate-limit", "fix/upload-retry", "hotfix/token", "docs/readme", "refactor/config"]
SERVICES = ["nginx", "postgres", "redis", "api", "worker", "web", "db", "cache", "queue", "cron", "haproxy"]
PKGS = ["jq", "ripgrep", "fd", "htop", "tmux", "curl", "pandas", "requests", "serde", "tokio", "react", "vite",
        "eslint", "prettier", "zod", "lodash", "numpy", "flask", "express", "typescript", "pytest", "black"]
WORDS = ["token", "upload", "session", "retry", "cache", "config", "parser", "limiter", "bucket", "quota", "login",
         "report", "invoice", "user", "order", "cart", "search", "index", "worker", "queue", "schema", "client",
         "server", "router", "handler", "export", "import", "auth", "billing", "metrics", "logger", "theme"]
EXTS = ["ts", "tsx", "js", "py", "rs", "go", "rb", "swift", "c", "h", "java", "kt", "sql", "md", "toml", "yaml", "json", "sh"]
DIRS = ["src", "src/api", "src/components", "lib", "tests", "scripts", "docs", "config", "app", "cmd/server",
        "internal/store", "pkg/util", "crates/core/src", "packages/ui/src", "migrations", "deploy", "assets"]
TOOLS = ["deployctl", "acmectl", "fluxion", "kvtool", "relay", "statusd", "gridctl", "tilekit", "metricsd", "shipit",
         "orbctl", "lintr", "bakery", "harbourctl", "notectl", "vaultish"]
SUBS = ["push", "status", "sync", "rollout", "init", "apply", "diff", "list", "get", "run", "check", "serve"]
MSGS = ["Fix token refresh race", "Add retry to upload client", "Bump parser to 2.4", "Split settings screen",
        "Reset quotas at midnight UTC", "Remove dead code", "Handle empty responses", "Speed up the search index",
        "Document the deploy flow", "Use the shared logger", "Tidy imports", "Rename the cart module"]
VARS = ["RUST_LOG", "NODE_ENV", "DATABASE_URL", "PORT", "DEBUG", "API_KEY", "PGPORT", "LOG_LEVEL", "CI", "GOFLAGS"]
FIRST = ["Ada", "Ben", "Cleo", "Dev", "Eli", "Fay", "Gus", "Hana", "Ivo", "Jun"]


def word():
    return pick(WORDS)


def proj():
    return pick(PROJECTS)


def ip():
    return pick(["192.0.2.", "198.51.100.", "203.0.113.", "10.42.1.", "10.0.0."]) + str(R.randrange(1, 255))


def host():
    return pick(["api", "auth", "db", "docs", "registry", "bastion", "gateway", "cdn", "mirror", "staging", "shop"]) + ".example.invalid"


def path(rel=True):
    p = pick(DIRS) + "/" + word() + pick(["", "_test", "-utils", "s", "_v2"]) + "." + pick(EXTS)
    if not rel or chance(0.15):
        return "/home/user/" + proj() + "/" + p
    return p


def hexhash(n=7):
    return "".join(pick("0123456789abcdef") for _ in range(n))


def uuid():
    return "-".join(hexhash(k) for k in (8, 4, 4, 4, 12))


def ver():
    return f"{R.randrange(0, 5)}.{R.randrange(0, 20)}.{R.randrange(0, 30)}"


def num(lo=1, hi=100):
    return str(R.randrange(lo, hi))


def stamp():
    return f"2026-{R.randrange(1, 13):02d}-{R.randrange(1, 29):02d}T{R.randrange(24):02d}:{R.randrange(60):02d}:{R.randrange(60):02d}Z"


def clock():
    return f"{R.randrange(24):02d}:{R.randrange(60):02d}:{R.randrange(60):02d}"


# ------------------------------------------------------------------ commands

def fill(t):
    reps = {
        "{n}": lambda: num(2, 30), "{path}": lambda: path(), "{branch}": lambda: pick(BRANCHES),
        "{msg}": lambda: pick(MSGS), "{hash}": lambda: hexhash(), "{ver}": ver, "{proj}": proj,
        "{crate}": proj, "{mod}": word, "{port}": lambda: pick(["3000", "8080", "5432", "6379", "8443", "9000", "4000"]),
        "{script}": lambda: pick(["report", "main", "seed", "migrate", "server", "train", "build", "sync"]),
        "{pkg}": lambda: pick(PKGS), "{test}": lambda: "test_" + word(), "{kw}": word,
        "{svc}": lambda: pick(SERVICES), "{registry}": lambda: "registry.example.invalid/team",
        "{image}": lambda: "registry.example.invalid/team/" + proj() + ":" + ver(), "{ns}": lambda: pick(["shop", "prod", "staging", "kube-system", "default"]),
        "{pod}": lambda: proj() + "-" + hexhash(10) + "-" + hexhash(5), "{file}": lambda: word() + pick(["", "s", "-old"]),
        "{dir}": lambda: pick(DIRS), "{ext}": lambda: pick(EXTS), "{log}": lambda: "/var/log/" + pick(SERVICES) + "/" + pick(["access", "error", "app"]) + ".log",
        "{host}": host, "{ip}": ip, "{user}": lambda: pick(USERS), "{p}": lambda: word() + "s",
        "{json}": lambda: '{"name":"' + word() + '","limit":' + num() + '}', "{VAR}": lambda: pick(VARS),
        "{val}": lambda: pick(["1", "debug", "production", "true", "8080", "postgres://app@db.example.invalid/app"]),
        "{pid}": lambda: num(1000, 99999), "{ctx}": lambda: pick(["prod", "staging", "kind-dev"]),
        "{env}": lambda: pick(["staging", "prod", "dev"]), "{a}": word, "{b}": word, "{Test}": lambda: "Test" + word().capitalize(),
        "{tool}": lambda: pick(TOOLS), "{sub}": lambda: pick(SUBS), "{flag}": lambda: pick(["name", "tag", "env", "region", "out", "limit"]),
    }
    out = t
    for k, f in reps.items():
        while k in out:
            out = out.replace(k, f(), 1)
    return out


FAMILIES = {
    "git": ["git status", "git status --short", "git log --oneline -{n}", "git log --graph --oneline --all", "git diff {path}",
            "git diff --stat main...HEAD", "git checkout -b {branch}", "git switch {branch}", 'git commit -m "{msg}"',
            "git commit --amend --no-edit", "git push origin {branch}", "git push -u origin {branch}", "git pull --rebase",
            "git fetch origin", "git rebase -i origin/main", 'git stash push -m "{msg}"', "git branch -a", "git branch",
            "git show {hash} --stat", "git add -p {path}", "git add .", "git restore --staged {path}",
            "git blame -L 10,30 {path}", "git tag v{ver}", "git cherry-pick {hash}", "git reset --hard HEAD~1",
            "git remote -v", "git worktree add ../{proj}-wt {branch}", "git log -1 --format=%H", "git diff"],
    "cargo": ["cargo build", "cargo build --release", "cargo test", "cargo test -p {crate}", "cargo test {mod}::tests",
              "cargo clippy --all-targets -- -D warnings", "cargo fmt --all", "cargo run --bin {crate} -- --port {port}",
              "cargo check -p {crate}", "cargo add serde --features derive", "cargo update -p {crate}", "cargo bench",
              "cargo test --workspace --quiet", "cargo nextest run"],
    "python": ["python3 {script}.py --month 2026-{n}", "python -m venv .venv", "python -m pytest -q", "pip install -r requirements.txt",
               "pip install {pkg}=={ver}", 'pytest -q tests/{test}.py -k "{kw}"', "pytest -x", "uv run {script}.py",
               "uv sync", "poetry install", "ruff check .", "mypy src", "python3 -c 'import {pkg}; print({pkg}.__version__)'",
               "pip list --outdated", "python manage.py migrate"],
    "docker": ["docker ps", "docker ps -a", "docker compose up -d {svc}", "docker compose logs -f {svc}",
               "docker build -t {registry}/{proj}:{ver} .", "docker run --rm -it -p {port}:{port} {image}",
               "docker exec -it {svc} sh", "docker images", "docker system prune -f", "docker pull {image}",
               "docker compose down -v", "docker logs --tail 20 {svc}"],
    "k8s": ["kubectl get pods", "kubectl -n {ns} get pods -o wide", "kubectl describe pod {pod}", "kubectl logs -f {pod}",
            "kubectl apply -f {file}.yaml", "kubectl rollout status deploy/{svc}", "kubectl port-forward svc/{svc} {port}:80",
            "helm upgrade --install {svc} ./charts/{svc} -f values.yaml", "kubectl config use-context {ctx}",
            "kubectl -n {ns} get svc", "kubectl get nodes"],
    "files": ["ls", "ls -la", "ls -l {dir}", "ls -lh", "cd {dir}", "cd ..", "cat {path}", "less {path}", "head -n 20 {path}",
              "tail -f {log}", "tail -n 50 {log}", 'grep -rn "{kw}" {dir}', "rg {kw} {dir}", 'find . -name "*.{ext}" -mtime -1',
              "fd -e {ext}", "wc -l {path}", "du -sh *", "df -h", "mkdir -p {dir}", "cp {path} /tmp/", "mv {file}.txt {dir}/",
              "rm -rf {dir}", "chmod +x scripts/{script}.sh", "touch {file}.md", "tree -L 2", "stat {path}", "cat {file}.json",
              "cat config.toml", "bat {path}", "ls -1 {dir}", "pwd", "cat Dockerfile", "cat .github/workflows/ci.yml",
              "cat scripts/{script}.sh", "less Dockerfile"],
    "net": ["curl -sS https://{host}/v1/{p} | jq .", "curl -I https://{host}", "wget https://{host}/{file}.tar.gz",
            "curl -X POST https://{host}/v2/{p} -H 'Content-Type: application/json' -d '{json}'",
            "ping -c 3 {host}", "dig +short {host}", "ssh {user}@{ip}", "ssh -L 8443:localhost:443 {user}@{host}",
            "scp {file}.txt {user}@{host}:/tmp/", "rsync -avz build/ {user}@{host}:/srv/{proj}", "nc -zv {host} {port}",
            "traceroute {host}", "curl -s localhost:{port}/healthz"],
    "sys": ["sudo systemctl restart {svc}", "systemctl status {svc} --no-pager", "journalctl -u {svc} -n 20 --no-pager",
            "sudo apt update && sudo apt upgrade -y", "sudo apt install -y {pkg}", "brew install {pkg}", "brew upgrade",
            "ps aux | grep {svc}", "kill -9 {pid}", "uptime", "free -h", "uname -a", "whoami", "env | grep {VAR}",
            "export {VAR}={val}", "source .venv/bin/activate", "which {pkg}", "sudo nginx -t", "crontab -l", "lsof -i :{port}",
            "history | tail -5", "date -u", "echo $PATH", "brew update && brew upgrade --greedy"],
    "build": ["make", "make -j8", "make test", "make clean all", "cmake -S . -B build -DCMAKE_BUILD_TYPE=Release",
              "cmake --build build -j", "go build ./...", "go test ./... -run {Test}", "go mod tidy", "just lint", "just test",
              "./gradlew build", "mvn -q package", "swift build -c release", "zig build test", "dotnet test", "go vet ./..."],
    "node": ["npm install", "npm ci", "npm run build", "npm run dev", "npm test", "npm test -- --watch=false src/{kw}",
             "npx vitest run {kw}", "npx prisma migrate deploy", "npx tsc --noEmit", "npx eslint src --fix",
             "pnpm install --frozen-lockfile", "pnpm -r build", "yarn add {pkg}", "node {script}.js", "bun run {script}.ts",
             "deno task dev", "npm run lint", "npm outdated"],
    "misc": ["jq '.items[] | .name' {file}.json", "sed -i 's/{a}/{b}/g' {path}", "awk -F, '{print $2}' {file}.csv | sort | uniq -c",
             "tar -czf {proj}.tar.gz {dir}", "unzip {file}.zip -d {dir}", "openssl x509 -in cert.pem -noout -enddate",
             "terraform plan -out tf.plan", "terraform apply tf.plan", "ansible-playbook -i inventory site.yml --check",
             "psql postgres://app@{host}/app -c 'select 1'", "redis-cli -h {host} ping", "sqlite3 {file}.db '.tables'",
             "ffmpeg -i {file}.mov -vf scale=1280:-1 {file}.mp4", "gh pr create --fill", "gh run watch", "tmux attach -t {proj}",
             "printf '%s\\n' {a} {b}", "watch -n 2 kubectl get pods", "terraform init", "gh pr checks"],
    "custom": ["./scripts/deploy.sh --dry-run {env}", "./scripts/setup.sh --with-fixtures", "./bin/migrate up", "~/bin/sync-{kw}",
               "{tool} {sub} --{flag} {kw}", "{tool} --help", "{tool} {sub}", "./run.sh", "bin/rails db:migrate",
               "./manage.py runserver 0.0.0.0:{port}", "{VAR}={val} ./scripts/{script}.sh", "{tool} {sub} -v",
               "./target/release/{crate} --config config.toml"],
}
FAMILY_NAMES = list(FAMILIES)
PIPE_TAILS = ["tail -20", "grep -v DEBUG", "wc -l", "jq .", "sort | uniq -c", "head -5", "less", "grep -i error",
              "tee /tmp/out.log", "column -t"]
TYPOS = ["gti status", "sl", "gut push", "carg build", "nmp install", "pyhton3 main.py", "dokcer ps", "claer", "mkae"]


def command(long=False):
    fam = pick(FAMILY_NAMES)
    cmd = fill(pick(FAMILIES[fam]))
    r = R.random()
    if r < 0.12:
        cmd += " && " + fill(pick(FAMILIES[pick(FAMILY_NAMES)]))
    elif r < 0.22 and "|" not in cmd:
        cmd += " | " + pick(PIPE_TAILS)
    elif r < 0.26:
        cmd += " 2>&1 | tail -" + num(5, 60)
    if chance(0.05) and not cmd.startswith("sudo"):
        cmd = "sudo " + cmd
    if chance(0.05):
        cmd = pick(VARS) + "=" + pick(["1", "debug", "test", "0"]) + " " + cmd
    if long or chance(0.06):
        extra = [fill(pick(FAMILIES[pick(FAMILY_NAMES)])) for _ in range(R.randrange(1, 4))]
        cmd = " && ".join([cmd] + extra)
    return cmd


# ------------------------------------------------------------------- outputs

def out_git_status():
    b = pick(BRANCHES)
    rows = [f"On branch {b}"]
    if chance(0.6):
        rows.append(f"Your branch is up to date with 'origin/{b}'.")
    rows.append("")
    if chance(0.7):
        rows += ["Changes not staged for commit:", '  (use "git add <file>..." to update what will be committed)']
        rows += [f"        modified:   {path()}" for _ in range(R.randrange(1, 4))]
        rows.append("")
    if chance(0.5):
        rows += ["Untracked files:", '  (use "git add <file>..." to include in what will be committed)']
        rows += [f"        {path()}" for _ in range(R.randrange(1, 3))]
    if len(rows) < 4:
        rows.append("nothing to commit, working tree clean")
    return rows


def out_git_log():
    return [f"{hexhash()} {pick(MSGS)}" for _ in range(R.randrange(3, 9))]


def out_git_diff():
    p = path()
    rows = [f"diff --git a/{p} b/{p}", f"index {hexhash()}..{hexhash()} 100644", f"--- a/{p}", f"+++ b/{p}",
            f"@@ -{num()},{num(3, 9)} +{num()},{num(3, 9)} @@ {pick(['fn', 'def', 'function', 'class', 'set -euo pipefail'])} {word()}"]
    body = [f"const {word().upper()}_LIMIT = {num()};", f"{word()} = {word()}({word()})", f"import {{ {word()} }} from \"./{word()}\";",
            fill("npm run build"), fill("{tool} {sub}"), "return nil", f"if err != nil {{", f"    log.Println(\"{word()}\")",
            fill(pick(FAMILIES["git"])), f"echo \"{word()}\"", "}", f"let {word()} = config.{word()};", fill("cargo test -p {crate}")]
    for _ in range(R.randrange(3, 9)):
        rows.append(pick([" ", "+", "-", " "]) + pick(["", "    ", "  "]) + pick(body))
    return rows


def out_ls_long():
    rows = [f"total {num(8, 200)}"]
    for _ in range(R.randrange(3, 10)):
        d = chance(0.3)
        perm = ("d" if d else "-") + pick(["rwxr-xr-x", "rw-r--r--", "rwxrwxr-x", "rw-------"])
        name = pick(DIRS).split("/")[-1] if d else word() + "." + pick(EXTS)
        rows.append(f"{perm}  {num(1, 20):>2} {pick(USERS)}  {pick(['staff', 'users', 'user', 'wheel'])}  {num(64, 99999):>6} "
                    f"{pick(['Nov', 'Oct', 'Jan', 'Mar'])} {num(1, 29):>2} {R.randrange(24):02d}:{R.randrange(60):02d} {name}")
    return rows


def out_ls_short():
    names = [word() + pick(["", "." + pick(EXTS)]) for _ in range(R.randrange(3, 12))]
    return ["  ".join(names)]


def out_cargo_build(errors=None):
    c = proj()
    rows = [f"   Compiling {pick(PKGS)} v{ver()}" for _ in range(R.randrange(0, 4))]
    rows.append(f"   Compiling {c} v{ver()} (/home/user/w/{c})")
    if errors is None:
        errors = chance(0.5)
    if errors:
        v = word()
        rows += [f"error[E0{num(100, 999)}]: cannot find value `{v}` in this scope",
                 f"  --> src/{word()}.rs:{num(1, 300)}:{num(1, 60)}", "   |",
                 f"{num(10, 99)} |         let n = {v}.take(1);", "   |                 ^^^^^^ not found in this scope", "",
                 f"error: could not compile `{c}` (bin \"{c}\") due to 1 previous error"]
    else:
        if chance(0.4):
            rows += [f"warning: unused variable: `{word()}`", f"  --> src/{word()}.rs:{num()}:{num()}", "   |",
                     f"   = note: `#[warn(unused_variables)]` on by default", ""]
        rows.append(f"    Finished `{pick(['dev', 'release'])}` profile [optimized] target(s) in {num(1, 60)}.{num(10, 99)}s")
    return rows


def out_test_rust():
    c = proj()
    rows = [f"    Finished `test` profile [unoptimized + debuginfo] target(s) in {num(1, 9)}.{num(10, 99)}s",
            f"     Running unittests src/lib.rs (target/debug/deps/{c}-{hexhash(16)})", ""]
    n = R.randrange(2, 9)
    rows.append(f"running {n} tests")
    failed = 0
    for _ in range(n):
        res = pick(["ok", "ok", "ok", "ok", "FAILED", "ignored"])
        failed += res == "FAILED"
        rows.append(f"test {word()}::tests::{word()}_{pick(['works', 'fails', 'refills', 'burst', 'empty', 'parses'])} ... {res}")
    rows.append("")
    status = "FAILED" if failed else "ok"
    rows.append(f"test result: {status}. {n - failed} passed; {failed} failed; 0 ignored; 0 measured; {num(0, 30)} filtered out; finished in 0.{num(10, 99)}s")
    return rows


def out_test_go():
    rows = []
    for _ in range(R.randrange(2, 6)):
        t = "Test" + word().capitalize()
        res = pick(["PASS", "PASS", "FAIL"])
        rows += [f"=== RUN   {t}", f"--- {res}: {t} (0.{num(10, 99)}s)"]
    rows.append(f"ok  \texample.invalid/{proj()}/{word()}\t0.{num(100, 999)}s".replace("\t", "    "))
    return rows


def out_jest():
    rows = [f"> {proj()}@{ver()} test", f"> {pick(['jest', 'vitest run', 'jest --ci'])}", ""]
    for _ in range(R.randrange(1, 4)):
        rows.append(f" {pick(['PASS', 'PASS', 'FAIL'])}  src/{word()}/{word()}.test.ts")
    if chance(0.5):
        rows += [f"  ● {word()} {pick(['applies the discount once', 'rejects empty input', 'retries on 503'])}", "",
                 "    expect(received).toBe(expected) // Object.is equality", "", f"    Expected: {num()}", f"    Received: {num()}", ""]
    rows += [f"Tests:       {num(0, 3)} failed, {num(3, 40)} passed, {num(10, 50)} total", f"Time:        {num(1, 9)}.{num(10, 99)} s"]
    return rows


def out_pytest():
    rows = [f"{'.' * R.randrange(2, 9)}{pick(['', 'F', 'F.'])}" + " " * R.randrange(20, 60) + "[100%]"]
    if chance(0.5):
        rows += ["=" * 30 + " FAILURES " + "=" * 30, "_" * 20 + f" test_{word()} " + "_" * 20,
                 f"    def test_{word()}():", f">       assert {word()}.total([1.005, 2.0]) == 3.01", "E       assert 3.0 == 3.01",
                 f"tests/test_{word()}.py:{num()}: AssertionError"]
    rows.append(f"{num(0, 3)} failed, {num(1, 30)} passed in 0.{num(10, 99)}s")
    return rows


def out_npm_install():
    rows = []
    if chance(0.4):
        rows.append(f"npm warn deprecated {pick(PKGS)}@{ver()}: This package is no longer supported.")
    rows += [f"added {num(10, 900)} packages, and audited {num(100, 999)} packages in {num(1, 30)}s", "",
             f"{num(10, 200)} packages are looking for funding", "  run `npm fund` for details", "",
             "found 0 vulnerabilities"]
    return rows


def out_npm_build():
    rows = [f"> {proj()}@{ver()} build", f"> {pick(['vite build', 'tsc && vite build', 'next build', 'webpack --mode production'])}", ""]
    rows += [f"✓ {num(10, 400)} modules transformed.",
             f"dist/assets/index-{hexhash(6)}.js   {num(10, 300)}.{num(10, 99)} kB │ gzip: {num(10, 90)}.{num(10, 99)} kB",
             f"✓ built in {num(1, 9)}.{num(10, 99)}s"]
    return rows


def out_pip():
    p = pick(PKGS)
    return [f"Collecting {p}=={ver()} (from -r requirements.txt (line {num(1, 9)}))",
            f"  Using cached {p}-{ver()}-py3-none-any.whl.metadata ({num(2, 90)} kB)",
            f"Requirement already satisfied: {pick(PKGS)}>={ver()} in ./.venv/lib/python3.12/site-packages ({ver()})",
            f"Installing collected packages: {p}", f"Successfully installed {p}-{ver()}"]


def out_make():
    rows = []
    for _ in range(R.randrange(1, 4)):
        f = word()
        rows.append(Cmd(f"cc -O2 -Wall -c -o src/{f}.o src/{f}.c"))
    if chance(0.6):
        f = word()
        rows += [f"src/{f}.c: In function '{word()}_{word()}':",
                 f"src/{f}.c:{num()}:{num(1, 30)}: {pick(['warning', 'error'])}: unused variable 'len' [-Wunused-variable]",
                 f"   {num(10, 99)} |     size_t len = strlen(line);", "      |             ^~~",
                 f"make: *** [Makefile:{num(5, 40)}: src/{f}.o] Error 1"]
    else:
        rows.append(Cmd(f"cc -o {proj()} src/*.o -lpthread"))
    return rows


def out_tsc():
    return [f"src/{word()}.ts({num()},{num(1, 40)}): error TS{num(2000, 2999)}: Type 'string' is not assignable to type 'number'."
            for _ in range(R.randrange(1, 4))] + [f"Found {num(1, 5)} errors in {num(1, 3)} files."]


def out_docker_ps():
    rows = ["CONTAINER ID   IMAGE                      COMMAND                  CREATED        STATUS        PORTS                    NAMES"]
    for _ in range(R.randrange(1, 5)):
        rows.append(f"{hexhash(12)}   {pick(SERVICES)}:{ver():<18}   \"{pick(['docker-entrypoint.s…', '/bin/sh -c', 'nginx -g daemon…'])}\"   "
                    f"{num(2, 50)} minutes ago   Up {num(2, 50)} minutes   0.0.0.0:{num(3000, 9000)}->80/tcp   {proj()}-{pick(SERVICES)}-1")
    return rows


def out_docker_build():
    rows = [f"[+] Building {num(1, 90)}.{num(1, 9)}s ({num(1, 12)}/{num(12, 14)})",
            " => [internal] load build definition from Dockerfile" + " " * 30 + "0.0s"]
    steps = ["FROM docker.io/library/python:3.12-slim", "RUN apt-get update && apt-get install -y curl",
             "WORKDIR /app", "COPY requirements.txt .", "RUN pip install --no-cache-dir -r requirements.txt",
             "COPY . .", "RUN npm ci && npm run build", "RUN python -m compileall -q ."]
    n = R.randrange(3, 7)
    for i in range(n):
        step = pick(steps)
        row = pick([f" => [{i + 1}/{n}] {step}", f"#{i + 4} [{i + 1}/{n}] {step}", f"Step {i + 1}/{n} : {step}"])
        if row.startswith(" =>"):
            row += " " * R.randrange(4, 30) + f"{num(0, 30)}.{num(0, 9)}s"
        rows.append(Cmd(row) if "RUN " in step else row)
    if chance(0.3):
        rows.append(f"ERROR: failed to solve: process \"/bin/sh -c {pick(steps)[4:]}\" did not complete successfully")
    return rows


def out_kubectl_get():
    rows = ["NAME                        READY   STATUS             RESTARTS   AGE   IP            NODE"]
    for _ in range(R.randrange(2, 6)):
        rows.append(f"{proj()}-{hexhash(10)}-{hexhash(5)}   {pick(['1/1', '0/1', '2/2'])}     "
                    f"{pick(['Running', 'Running', 'CrashLoopBackOff', 'Pending']):<17}  {num(0, 20):<9}  {num(1, 60)}m   {ip():<13} node-{pick('abc')}")
    return rows


def out_logs():
    rows = []
    for _ in range(R.randrange(3, 10)):
        lvl = pick(["INFO ", "INFO ", "WARN ", "ERROR", "DEBUG"])
        rows.append(f"{stamp()} {lvl} " + pick([
            f"req={uuid()} ip={ip()} GET /api/{word()}s {pick(['200', '404', '500', '429'])} {num(1, 900)}ms",
            f"upstream https://{host()}/token timed out after {num(100, 5000)}ms",
            f"retrying in {num(1, 9)}s (attempt {num(1, 5)}/5)", f"{word()} worker started on port {num(3000, 9000)}",
            f"connected to postgres://{host()}:5432/app", f"cache miss for key {word()}:{num()}"]))
    return rows


def out_journal():
    s = pick(SERVICES)
    return [f"Nov {num(1, 29)} {clock()} {pick(HOSTS)} {pick(['systemd[1]', s + '[' + num(1000, 99999) + ']'])}: "
            + pick([f"Started {s}.service - {s} server.", f"Reloading {s}.service...", f"{s}: worker process {num(1000, 9999)} exited",
                    f"Stopped {s}.service.", f"listening on 0.0.0.0:{num(1000, 9000)}"]) for _ in range(R.randrange(2, 7))]


def out_json():
    rows = ["{"]
    keys = R.randrange(2, 6)
    for i in range(keys):
        k = word()
        v = pick([f'"{uuid()}"', f'"{word()}"', num(), "true", "null", f'"https://{host()}/v2/{word()}s"'])
        rows.append(f'  "{k}": {v}' + ("," if i < keys - 1 else ""))
    rows.append("}")
    return rows


def out_headers():
    return ["HTTP/2 " + pick(["200", "301", "404"]), "content-type: text/html; charset=utf-8", f"date: Wed, 12 Nov 2026 {clock()} GMT",
            f"server: {pick(['nginx', 'envoy', 'cloudfront'])}", f"x-request-id: {uuid()}"]


def out_ping():
    h = host()
    a = ip()
    rows = [f"PING {h} ({a}): 56 data bytes"]
    rows += [f"64 bytes from {a}: icmp_seq={i} ttl=64 time={num(1, 40)}.{num(100, 999)} ms" for i in range(R.randrange(2, 5))]
    rows += ["", f"--- {h} ping statistics ---", "3 packets transmitted, 3 packets received, 0.0% packet loss"]
    return rows


def out_grep():
    return [f"{path()}:{num(1, 300)}:{pick(['  ', '    ', ''])}{pick(['const ', 'let ', '// TODO: ', 'def ', 'fn ', ''])}{word()}{pick(['()', ' = 3', ' = config.' + word(), ': ' + word()])}"
            for _ in range(R.randrange(1, 7))]


def out_help():
    t = pick(TOOLS)
    rows = [pick([f"{t.capitalize()} manages {word()}s for the example cluster.", f"A small tool to {word()} {word()}s.", f"{t} {ver()}"]), "",
            f"Usage: {t} [OPTIONS] <COMMAND>", "", "Commands:"]
    for s in R.sample(SUBS, R.randrange(2, 5)):
        rows.append(f"  {s:<9}{pick(['Build and push an image', 'Show status', 'Sync the local copy', 'Print help', 'Roll out a change'])}")
    rows += ["", "Options:", "  -c, --config <FILE>   Config file [default: config.toml]", "  -v, --verbose         Print more detail",
             "  -h, --help            Print help"]
    ex = []
    if chance(0.5):
        ex = [("T", ""), ("O", "Examples:")] + [("K", "  " + fill(f"{t} {{sub}} --{{flag}} {{kw}}"))]
    return rows, ex


def out_history():
    return [f"  {num(100, 999)}  {command()}" for _ in range(R.randrange(3, 8))]


def out_env():
    return [f"{v}={fill('{val}')}" for v in R.sample(VARS, R.randrange(2, 6))]


def out_systemctl():
    s = pick(SERVICES)
    return [f"● {s}.service - {s} server", f"     Loaded: loaded (/lib/systemd/system/{s}.service; enabled; preset: enabled)",
            f"     Active: active (running) since Mon 2026-11-03 {clock()} UTC; {num(1, 9)}h ago",
            f"   Main PID: {num(1000, 99999)} ({s})", f"      Tasks: {num(1, 20)} (limit: 4915)"]


def out_df():
    rows = ["Filesystem      Size  Used Avail Use% Mounted on"]
    for m in ["/", "/boot", "/srv", "/home"][: R.randrange(2, 5)]:
        rows.append(f"/dev/sda{num(1, 4)}       {num(10, 900)}G  {num(1, 400)}G  {num(1, 400)}G  {num(1, 99)}% {m}")
    return rows


def out_brew():
    p = pick(PKGS)
    return ["==> Updating Homebrew...", f"==> Downloading https://ghcr.io/v2/homebrew/core/{p}/manifests/{ver()}",
            f"==> Pouring {p}--{ver()}.arm64_sequoia.bottle.tar.gz", f"🍺  /opt/homebrew/Cellar/{p}/{ver()}: {num(5, 90)} files, {num(1, 9)}.{num(1, 9)}MB"]


def out_apt():
    return ["Hit:1 http://deb.example.invalid/debian stable InRelease", "Reading package lists... Done",
            "Building dependency tree... Done", f"{num(0, 30)} upgraded, {num(0, 5)} newly installed, 0 to remove and 0 not upgraded."]


def out_terraform():
    rows = ["Terraform will perform the following actions:", ""]
    for _ in range(R.randrange(1, 3)):
        rows += [f"  # aws_instance.{word()} will be created", f'  + resource "aws_instance" "{word()}" {{', f'      + ami           = "ami-{hexhash(8)}"',
                 f'      + instance_type = "t3.{pick(["micro", "small"])}"', "    }", ""]
    rows.append(f"Plan: {num(1, 4)} to add, 0 to change, 0 to destroy.")
    return rows


def out_traceback():
    return ["Traceback (most recent call last):", f'  File "/home/user/{proj()}/{word()}.py", line {num()}, in <module>', "    main()",
            f'  File "/home/user/{proj()}/lib/{word()}.py", line {num()}, in {word()}', f"    rows = load_rows(args.{word()})",
            f"FileNotFoundError: [Errno 2] No such file or directory: 'data/{word()}.csv'"]


def out_not_found(cmd):
    w = cmd.split()[0]
    return [pick([f"zsh: command not found: {w}", f"bash: {w}: command not found", f"fish: Unknown command: {w}"])]


def comment():
    return (pick(["# ", "#", "// ", "-- "]) + pick([f"{word().capitalize()} settings", f"Set this to the {word()} host", "TODO: remove after the migration",
                                                 f"Generated by {pick(TOOLS)} {ver()}", f"{pick(['p', 'pick', 'r', 'reword'])} <commit> = use commit",
                                                 f"{pick(TOOLS)} report", "Do not edit by hand",
                                                 f"Rebase {hexhash()}..{hexhash()} onto {hexhash()} ({num(1, 9)} commands)", ""])).rstrip()


def out_cat_config():
    rows = [f"[{word()}]", f'host = "{pick(["0.0.0.0", "127.0.0.1"])}"', f"port = {num(1000, 9000)}", f'upstream = "https://{host()}"', "",
            f"[{word()}]", f'level = "{pick(["info", "debug"])}"', f"retries = {num(1, 9)}"]
    rows = rows[: R.randrange(3, len(rows))]
    for _ in range(R.randrange(0, 3)):
        rows.insert(R.randrange(0, len(rows) + 1), comment())
    return rows


def out_cat_script():
    rows = [pick(["#!/usr/bin/env bash", "#!/bin/sh", "#!/usr/bin/env zsh"]), comment() or "#",
            Cmd(pick(["set -euo pipefail", "set -e", 'cd "$(dirname "$0")"'])), ""]
    for _ in range(R.randrange(2, 7)):
        indent = pick(["", "  ", "    "])
        r = R.random()
        if r < 0.2:
            rows.append(indent + (comment() or "#"))
        elif r < 0.3:
            rows.append(indent + pick(["fi", "done", "else", "}", "esac"]))
        else:
            rows.append(Cmd(indent + pick([command(), f"{word()}=$({command().split(' && ')[0]})", f"if [ -f {path()} ]; then",
                                           f"for f in {pick(DIRS)}/*; do", f'echo "{word()} done"', command().split(" && ")[0]])))
    return rows


def out_cat_dockerfile():
    rows = [f"FROM {pick(['python:3.12-slim', 'node:22-alpine', 'golang:1.23', 'debian:bookworm'])}", "WORKDIR /app"]
    for _ in range(R.randrange(1, 5)):
        if chance(0.6):
            rows.append(Cmd("RUN " + pick(["apt-get update && apt-get install -y --no-install-recommends curl", "pip install --no-cache-dir -r requirements.txt",
                                           "npm ci && npm run build", "go build -o /out/server ./cmd/server", fill("chmod +x scripts/{script}.sh")])))
        else:
            rows.append(pick(["COPY . .", "COPY requirements.txt .", f"ENV {pick(VARS)}={fill('{val}')}", f"EXPOSE {num(3000, 9000)}"]))
    rows.append(pick(['CMD ["python", "main.py"]', 'ENTRYPOINT ["/out/server"]', 'CMD ["npm", "start"]']))
    return rows


def out_cat_workflow():
    rows = ["name: ci", "on: [push, pull_request]", "jobs:", "  test:", "    runs-on: ubuntu-latest", "    steps:",
            "      - uses: actions/checkout@v4"]
    for _ in range(R.randrange(1, 4)):
        rows.append(Cmd("      - run: " + command().split(" && ")[0]) if chance(0.8) else f"      - uses: actions/setup-node@v4")
    return rows


def out_hint():
    intro = pick(["To address all issues, run:", "To start it now and restart at login:", "To configure your current shell, run:",
                  "Run the following to apply pending changes:", "Next steps:", "You can now run:", "To fix this, run:",
                  "Then run:", "Or, if you don't want/need a background service you can just run:", "To update, run:"])
    rows = [("O", pick(["", "==> Caveats", f"{num(1, 9)} {pick(['moderate', 'high'])} severity vulnerabilities", f"Installed {pick(PKGS)} {ver()}"])),
            ("O", intro)]
    for _ in range(R.randrange(1, 3)):
        rows.append(("K", pick(["  ", "    "]) + command()))
    return rows


def out_banner():
    t = pick([f"PLAY [{pick(SERVICES)}]", f"TASK [{word()} : {pick(['install packages', 'copy config', 'restart service'])}]", "PLAY RECAP",
              f"=== {word()} ===", f"----- Ran {num(1, 40)} tests in 0.{num(1, 99)}s -----", f"[{word().upper()}]"])
    rows = [t + " " + "*" * R.randrange(10, 80) if t.startswith(("PLAY", "TASK")) else t]
    rows += [pick([f"ok: [{pick(HOSTS)}.example.invalid]", f"changed: [{pick(HOSTS)}.example.invalid] => (item={pick(PKGS)})",
                   f"{pick(HOSTS)}.example.invalid : ok={num(1, 9)}    changed={num(0, 5)}    unreachable=0    failed=0"])
             for _ in range(R.randrange(1, 3))]
    return rows


def out_confirm():
    return [pick(["Do you want to continue? [Y/n] y", "Proceed (y/N)? y", "  Enter a value: yes",
                  "Are you sure you want to continue connecting (yes/no/[fingerprint])? yes", f"[sudo] password for {pick(USERS)}:",
                  "Do you want to perform these actions?", "  Only 'yes' will be accepted to approve.", "Is this ok [y/N]: y",
                  f"? Select a {word()} › {word()}", "Overwrite? (y/n [n]) n"])]


def out_node_trace():
    rows = [f"node:internal/modules/cjs/loader:{num(1000, 1300)}", "  throw err;", "  ^", "", f"Error: Cannot find module '{pick(PKGS)}'"]
    rows += [f"    at {pick(['Module._resolveFilename', 'Module._load', 'Object.<anonymous>', 'process.processTicksAndRejections'])} "
             f"({pick(['node:internal/modules/cjs/loader', '/home/user/' + proj() + '/server.js', 'node:internal/process/task_queues'])}:{num()}:{num()})"
             for _ in range(R.randrange(2, 5))]
    return rows + [f"Node.js v{num(18, 23)}.{num(1, 12)}.0"]


def out_go_panic():
    return ["panic: runtime error: invalid memory address or nil pointer dereference", "[signal SIGSEGV: segmentation violation code=0x1 addr=0x0]", "",
            f"goroutine {num(1, 90)} [running]:", f"example.invalid/{proj()}/internal/{word()}.(*{word().capitalize()}).Get(0x0)",
            f"        /home/user/w/{proj()}/internal/{word()}/{word()}.go:{num()} +0x{hexhash(2)}", "exit status 2"]


def out_generic():
    if chance(0.25):
        return pick([out_banner, out_confirm, out_node_trace, out_go_panic, out_cat_script])()
    return [pick([f"{word().capitalize()} {word()}s: {num()}", f"Done in {num(1, 9)}.{num(1, 99)}s", f"{num()} files changed",
                  f"Wrote {path()}", f"Listening on http://localhost:{num(3000, 9000)}", f"Already up to date.",
                  f"Synced {num()} {word()}s to {host()}", f"[{clock()}] {word()} ok", f"warning: {word()} is deprecated"])
            for _ in range(R.randrange(0, 5))]


def output_for(cmd):
    """Returns (rows, extra) where extra holds labelled rows (help examples)."""
    w = cmd.split()
    first = w[0] if w else ""
    if first in ("sudo",) and len(w) > 1:
        first = w[1]
    c = cmd
    if any(c.startswith(t) for t in TYPOS) or first in [t.split()[0] for t in TYPOS]:
        return out_not_found(cmd), []
    if c.startswith(("cd ", "export ", "source ", "mkdir ", "touch ", "cp ", "mv ", "chmod ", "git add", "git switch", "git checkout",
                     "kubectl config", "rm ")) or c in ("cd ..",):
        return [], []
    if "--help" in c:
        return out_help()
    if chance(0.08) and not c.startswith(("cd ", "ls")):
        rows, _ = output_for_plain(c)
        return rows[: R.randrange(0, 6)], out_hint()
    if c.startswith(("cat ", "bat ", "less ")) and "Dockerfile" in c:
        return out_cat_dockerfile(), []
    if c.startswith(("cat ", "bat ", "less ")) and (".yml" in c or ".yaml" in c):
        return out_cat_workflow(), []
    if c.startswith(("cat ", "bat ", "less ")) and (".sh" in c or chance(0.2)):
        return out_cat_script(), []
    if c.startswith(("node ", "npm start", "bun ")) and chance(0.4):
        return out_node_trace(), []
    if c.startswith(("go run", "go test")) and chance(0.3):
        return out_go_panic(), []
    if c.startswith(("ansible", "make test")) and chance(0.6):
        return out_banner() + out_banner(), []
    if c.startswith(("sudo apt", "terraform apply", "ssh ", "rm -i", "yum", "dnf")) and chance(0.5):
        rows, extra = output_for_plain(c)
        return out_confirm() + rows, extra
    return output_for_plain(c)


def output_for_plain(c):
    w = c.split()
    first = w[0] if w else ""
    if first in ("sudo",) and len(w) > 1:
        first = w[1]
    table = [("git status", out_git_status), ("git log", out_git_log), ("git diff", out_git_diff), ("git show", out_git_diff),
             ("git branch", lambda: [f"  {b}" if i else f"* {b}" for i, b in enumerate(R.sample(BRANCHES, 4))]),
             ("git push", lambda: [f"To https://git.example.invalid/team/{proj()}.git", f"   {hexhash()}..{hexhash()}  {pick(BRANCHES)} -> {pick(BRANCHES)}"]),
             ("git pull", lambda: ["Already up to date."]), ("git fetch", lambda: []),
             ("git commit", lambda: [f"[{pick(BRANCHES)} {hexhash()}] {pick(MSGS)}", f" {num(1, 9)} files changed, {num()} insertions(+), {num()} deletions(-)"]),
             ("ls -l", out_ls_long), ("ls -la", out_ls_long), ("ls -lh", out_ls_long), ("ls", out_ls_short),
             ("cargo test", out_test_rust), ("cargo nextest", out_test_rust), ("cargo build", out_cargo_build), ("cargo check", out_cargo_build),
             ("cargo clippy", out_cargo_build), ("cargo run", out_cargo_build), ("go test", out_test_go),
             ("npm test", out_jest), ("npx vitest", out_jest), ("npm install", out_npm_install), ("npm ci", out_npm_install),
             ("pnpm install", out_npm_install), ("yarn add", out_npm_install), ("npm run", out_npm_build), ("pnpm -r", out_npm_build),
             ("pytest", out_pytest), ("python -m pytest", out_pytest), ("pip install", out_pip), ("make", out_make),
             ("npx tsc", out_tsc), ("docker ps", out_docker_ps), ("docker build", out_docker_build), ("kubectl get", out_kubectl_get),
             ("kubectl -n", out_kubectl_get), ("kubectl logs", out_logs), ("tail", out_logs), ("docker compose logs", out_logs),
             ("docker logs", out_logs), ("journalctl", out_journal), ("curl -I", out_headers), ("curl", out_json),
             ("cat config", out_cat_config), ("cat", lambda: out_json() if ".json" in c else out_cat_config()), ("jq", out_json),
             ("ping", out_ping), ("grep", out_grep), ("rg", out_grep), ("history", out_history), ("env", out_env),
             ("systemctl status", out_systemctl), ("df", out_df), ("du", out_df), ("free", out_df), ("brew", out_brew),
             ("sudo apt", out_apt), ("apt", out_apt), ("terraform plan", out_terraform), ("python3", lambda: out_traceback() if chance(0.5) else out_generic()),
             ("dig", lambda: [ip() for _ in range(R.randrange(1, 3))]), ("which", lambda: [f"/opt/homebrew/bin/{first}"]),
             ("whoami", lambda: [pick(USERS)]), ("uname", lambda: [f"Linux {pick(HOSTS)} 6.{num(1, 12)}.0-{num(1, 40)}-generic #1 SMP x86_64 GNU/Linux"]),
             ("uptime", lambda: [f" {clock()} up {num(1, 90)} days,  {num(1, 9)} users,  load average: 0.{num(10, 99)}, 0.{num(10, 99)}, 0.{num(10, 99)}"]),
             ("echo", lambda: [pick(["/usr/local/bin:/usr/bin:/bin", word(), f"{word()} {word()}"])]), ("date", lambda: [f"Wed Nov 12 {clock()} UTC 2026"]),
             ("pwd", lambda: [f"/home/user/{proj()}"]), ("wc", lambda: [f"  {num(1, 900)} {path()}"])]
    for prefix, fn in table:
        if c.startswith(prefix) or first == prefix:
            return fn(), []
    return out_generic(), []


# ------------------------------------------------------------------- prompts

class Prompt:
    def __init__(self):
        self.style = pick(["bash"] * 6 + ["bash_plain", "bash_version", "rhel", "zsh"] + ["zsh"] * 2 + ["omz"] * 3 + ["fish"] * 2
                          + ["starship"] * 3 + ["p10k"] * 2 + ["venv", "root", "root", "pwsh", "minimal", "zsh_host"])
        self.user = "root" if self.style == "root" else pick(USERS)
        self.host = pick(HOSTS)
        self.proj = proj()
        self.dir = pick(["~", f"~/src/{self.proj}", f"~/w/{self.proj}", f"/srv/{self.proj}", f"~/{self.proj}", "/etc/nginx", f"~/code/{self.proj}"])
        self.branch = pick(BRANCHES)
        self.rprompt = chance(0.1)
        self.venv = pick(["(venv) ", "(.venv) ", f"({self.proj}) "])

    def base(self):
        return self.dir.rstrip("/").split("/")[-1] or "/"

    def header(self):
        """Rows above the input line (two-line prompts), label T."""
        if self.style == "starship":
            parts = [self.base()]
            if chance(0.8):
                parts.append(f"on  {self.branch}" + pick(["", " [!]", " [?]", " [!+]", " [⇡1]"]))
            if chance(0.5):
                parts.append(pick([f"is 📦 v{ver()}", f"via 🦀 v1.{num(70, 90)}.0", f"via  v{num(18, 23)}.{num(1, 9)}.0", f"via 🐍 v3.12.{num(1, 9)}"]))
            if chance(0.3):
                parts.append(f"took {num(2, 59)}s")
            return [" ".join(parts)] if chance(0.8) else ["", " ".join(parts)]
        if self.style == "p10k":
            left = f"╭─ {self.dir}  {self.branch}" + pick(["", " ⇡1", " !2", " ?1", " ⇡1 !2"])
            right = pick(["✔", "✘ 1", "✔ 3s", "✘ 127"]) + "  " + clock()
            return [(left, right)]
        return []

    def line(self):
        s = self.style
        if s == "bash":
            return f"{self.user}@{self.host}:{self.dir}$ "
        if s == "bash_plain":
            return "$ "
        if s == "bash_version":
            return pick(["bash-5.2$ ", "sh-3.2$ ", "bash-3.2$ "])
        if s == "rhel":
            return f"[{self.user}@{self.host} {self.base()}]$ "
        if s == "zsh":
            return f"{self.user}@{self.host} {self.base()} % "
        if s == "zsh_host":
            return f"{self.host}% "
        if s == "omz":
            git = pick([f" git:({self.branch}) ✗", f" git:({self.branch})", ""])
            return f"➜  {self.base()}{git} "
        if s == "fish":
            short = "/".join(p[:1] if i < len(self.dir.split('/')) - 1 else p for i, p in enumerate(self.dir.split("/")))
            br = pick([f" ({self.branch})", ""])
            return f"{self.user}@{self.host} {short}{br}> "
        if s == "starship":
            return pick(["❯ ", "❯ ", "λ ", "➜ "])
        if s == "p10k":
            return "╰─❯ "
        if s == "venv":
            return f"{self.venv}{self.user}@{self.host}:{self.dir}$ "
        if s == "root":
            return f"root@{self.host}:{self.dir}# "
        if s == "pwsh":
            return f"PS C:\\Users\\{self.user}\\{self.proj}> "
        if s == "minimal":
            return pick(["» ", "→ ", "λ ", "% ", "> "])
        return "$ "


def ps2_word(kind):
    return pick(["> ", "> "]) if kind == "bash" else {"quote": "dquote> ", "heredoc": "heredoc> ", "pipe": "pipe> ", "and": "cmdand> ", "bs": "> "}.get(kind, "> ")


# -------------------------------------------------------------------- screen

class Out:
    def __init__(self, width):
        self.w = width
        self.rows = []  # (label, text, soft)

    def add(self, label, text, soft=False):
        self.rows.append((label, text.rstrip() if label != "T" or text.strip() else "", soft))

    def blank(self):
        self.rows.append(("B", "", False))

    def wrapped(self, label, text, cont_label=None):
        """Terminal soft wrap: rows of exactly `w` cells joined by the wrap flag."""
        cont_label = cont_label or label
        text = text.rstrip()
        if len(text) <= self.w:
            self.add(label, text)
            return
        chunks = [text[i:i + self.w] for i in range(0, len(text), self.w)]
        for i, ch in enumerate(chunks):
            lab = label if i == 0 else cont_label
            soft = i < len(chunks) - 1
            self.rows.append((lab, ch, soft))

    def output(self, lines):
        for line in lines:
            if not line.strip():
                self.blank()
            elif isinstance(line, Cmd):
                self.wrapped("K", line, "c")
            else:
                self.wrapped("O", line)


def shell_entry(o, prompt, typed=None, bare_end=False):
    for h in prompt.header():
        if isinstance(h, tuple):
            left, right = h
            fillw = max(1, o.w - len(left) - len(right) - 2)
            o.add("T", left + " " + pick(["·", "─", " "]) * fillw + " " + right if o.w > len(left) + len(right) + 4 else left)
        elif h == "":
            o.blank()
        else:
            o.add("T", h[: o.w])
    line = prompt.line()
    if bare_end:
        o.add("T", line.rstrip() if chance(0.7) else line)
        return
    kind = R.random()
    zsh = prompt.style in ("zsh", "omz", "zsh_host", "p10k", "starship")
    if typed is None and kind < 0.07:
        # Backslash continuation.
        parts = [fill(pick(["docker run --rm -it", "rsync -avz --delete", "curl -sS -X POST https://{host}/v2/{p}",
                            "kubectl run debug --rm -it", "aws s3 sync ./dist s3://{proj}-site", "ffmpeg -i {file}.mov"]))]
        for _ in range(R.randrange(1, 4)):
            parts.append(fill(pick(["-p {port}:{port}", "-v /home/user/{proj}/config:/etc/{proj}:ro", "-e {VAR}={val}",
                                    "--image=busybox", "-H 'Content-Type: application/json'", "--exclude node_modules",
                                    "-d '{json}'", "{image}", "--restart unless-stopped", "--name {proj}"])))
        o.add("C", line + parts[0] + " \\")
        ps = "> "
        for i, p in enumerate(parts[1:]):
            last = i == len(parts) - 2
            o.add("c", ps + pick(["", "  ", "    "]) + p + ("" if last else " \\"))
        o.output(output_for(parts[0])[0][:4])
        return
    if typed is None and kind < 0.11:
        # Heredoc.
        target = pick(["config.toml", ".env.local", "notes.md", "/tmp/payload.json", "Dockerfile"])
        word_ = pick(["EOF", "EOF", "END", "'EOF'"])
        o.add("C", line + pick([f"cat <<{word_} > {target}", f"cat > {target} <<{word_}", f"tee {target} <<{word_}"]))
        ps = "heredoc> " if zsh else "> "
        body = pick([out_cat_config(), out_env(), out_json(), [f"FROM {pick(SERVICES)}:{ver()}", f"RUN {fill(pick(FAMILIES['sys']))}", "COPY . /app"]])
        for b in body[: R.randrange(1, 6)]:
            o.add("c", ps + b if b else ps.rstrip())
        o.add("c", ps + word_.strip("'"))
        return
    if typed is None and kind < 0.14:
        # Open quote.
        o.add("C", line + pick(['git commit -m "' + pick(MSGS), 'echo "first line', "psql -c 'select *", 'printf "%s\\n" "one']))
        o.add("c", (pick(["dquote> ", "quote> "]) if zsh else "> ") + pick(["second line\"", "from orders limit 5'", f"{word()} {word()}\"", 'two"']))
        return
    if typed is None and kind < 0.17:
        # Pipe or && continuation over two lines.
        o.add("C", line + fill(pick(["cat {log} |", "git log --oneline |", "make build &&", "ps aux |"])))
        o.add("c", (pick(["pipe> ", "cmdand> "]) if zsh else "> ") + pick(["grep -c error", "head -5", "make test", "grep " + word()]))
        return
    cmd = typed if typed is not None else (pick(TYPOS) if chance(0.03) else command())
    if prompt.style == "pwsh":
        cmd = pick(["Get-ChildItem -Force", "Get-Content .\\" + word() + ".log -Tail 20", "Set-Location ..\\" + proj(),
                    "Invoke-WebRequest https://" + host() + " -OutFile x.html", "dir", "ipconfig /all", "git status", "npm run build"])
    full = line + cmd
    if prompt.rprompt and len(full) + 12 < o.w:
        full = full + " " * (o.w - len(full) - 9) + clock()
    o.wrapped("C", full, "c")
    out, extra = output_for(cmd)
    if prompt.style == "pwsh" and cmd.startswith(("Get-ChildItem", "dir")):
        out = ["", f"    Directory: C:\\Users\\{prompt.user}\\{prompt.proj}", "", "Mode                 LastWriteTime         Length Name",
               "----                 -------------         ------ ----"] + [f"-a---          11/12/2026  {R.randrange(1, 12)}:{R.randrange(60):02d} AM   {num(100, 99999):>12} {word()}.{pick(EXTS)}" for _ in range(3)]
    if chance(0.5) or len(out) < 40:
        o.output(out[: R.randrange(1, 30)] if out else [])
    for lab, text in extra:
        if lab == "T" or not text:
            o.blank()
        else:
            o.add(lab, text)
    if cmd.startswith("ssh ") and "@" in cmd and chance(0.8):
        o.add("O", f"Last login: Mon Nov  3 {clock()} 2026 from {ip()}")
        prompt.user = cmd.split()[1].split("@")[0] if "@" in cmd.split()[1] else prompt.user
        prompt.host = pick(HOSTS)
        prompt.style = "bash"


def shell_session(o):
    p = Prompt()
    n = R.randrange(2, 9)
    for i in range(n):
        shell_entry(o, p)
        if chance(0.1):
            o.blank()
    r = R.random()
    if r < 0.6:
        shell_entry(o, p, bare_end=True)
    elif r < 0.8:
        shell_entry(o, p, typed=command())
    if chance(0.12):
        o.add("T", f"[{pick(PROJECTS)}] 0:{pick(['zsh', 'bash', 'fish'])}* 1:{pick(['vim', 'logs', 'server'])}-" + " " * max(1, o.w - 60) + f"\"{p.host}\" {clock()[:5]} 12-Nov-26")


# --------------------------------------------------------------------- prose

OPENERS = ["I'll", "Let me", "Now I'll", "Next, I'll", "I'm going to", "First I'll", "Then I'll", "I want to"]
ACTS = ["look at", "check", "update", "read", "fix", "rerun", "trace", "simplify", "move", "split", "rename", "test"]
THINGS = ["the {w} handler", "the {w} client", "the failing test", "the config loader", "the retry logic", "the {w} module",
          "the migration", "the lockfile", "the deploy script", "the build", "the {w} cache", "the parser", "the CI job",
          "the {w} endpoint", "the error path", "the watcher", "the schema", "the {w} queue", "the settings page"]
REASONS = ["because the token expires early", "since the column already exists", "so the reload stays under a second",
           "because two workers write the same file", "since the port is already taken", "so the tests stop flaking",
           "because the lockfile is out of date", "to see when the regression came in", "before we touch the API",
           "so the numbers line up with the old report", "which is what keeps the old symbols around"]
STATEMENTS = ["The {w} change touches the same code path.", "Both suites pass.", "That's the culprit.", "Done.",
              "Found it.", "The fix is small.", "It still loops when the cache is cold.", "Nothing else depends on it.",
              "The build is green again.", "This only happens on the first request.", "The {w} test covers the edge case now.",
              "All {n} tests pass.", "There are {n} call sites, all in {d}.", "The numbers match now.",
              "You need Docker and a recent Node.", "The API reads its settings from {p}.", "See the troubleshooting page for more.",
              "If the error persists, check that the port is free and try again.", "This section explains how to configure the server.",
              "The setup script writes a local config to {p}.", "Full docs live at https://{h}/{w}/setup.",
              "Most of the time goes into the {w} step.", "The retry limit now comes from config.", "Nothing here needs a restart."]


def inline_cmd():
    c = fill(pick(FAMILIES[pick(FAMILY_NAMES)]))
    return c if len(c) < 50 else c.split(" && ")[0][:50].rsplit(" ", 1)[0]


def sentence():
    r = R.random()
    if r < 0.3:
        s = f"{pick(OPENERS)} {pick(ACTS)} {pick(THINGS)} {pick(REASONS)}."
    elif r < 0.55:
        s = pick(STATEMENTS)
    elif r < 0.7:
        s = f"To {pick(['try it', 'check it', 'reset the data', 'see the logs', 'rebuild', 'run it locally'])}, run `{inline_cmd()}`{pick(['', ' and then ' + '`' + inline_cmd() + '`', ' first', ' again'])}."
    elif r < 0.8:
        s = f"{pick(['Run', 'Then run', 'Use', 'Try'])} {inline_cmd()} {pick(['and restart the API.', 'to confirm the fix.', 'before you commit.', 'if it still fails.'])}"
    elif r < 0.9:
        s = f"The {pick(['problem', 'bug', 'slowdown', 'failure'])} is in {path()}{pick(['', ' around line ' + num()])}."
    else:
        s = f"{pick(['Next', 'After that', 'Finally', 'Meanwhile'])}, {pick(ACTS)} {pick(THINGS)} {pick(REASONS)}."
    return s.replace("{w}", word()).replace("{n}", num(2, 40)).replace("{d}", pick(DIRS)).replace("{p}", path()).replace("{h}", host())


def paragraph(k=None):
    return " ".join(sentence() for _ in range(k or R.randrange(1, 4)))


def wrap(text, width, first, rest):
    return textwrap.wrap(text, width=max(20, width), initial_indent=first, subsequent_indent=rest,
                         break_long_words=False, break_on_hyphens=False) or [first.rstrip()]


# ---------------------------------------------------------------- Claude Code

BULLETS = ["●", "●", "⏺"]


def cc_bullet():
    return pick(BULLETS)


def cc_prose(o, text=None):
    for i, line in enumerate(wrap(text or paragraph(), o.w - 1, cc_bullet() + " ", "  ")):
        o.add("P", line)


def cc_bash(o, cmd=None):
    cmd = cmd or command(long=chance(0.3))
    if chance(0.25):
        cmd += pick([" 2>&1 | tail -" + num(10, 60), " 2>&1", " | head -" + num(5, 40)])
    lines = wrap(f"Bash({cmd})", o.w - 2, cc_bullet() + " ", "      ")
    for i, l in enumerate(lines):
        o.add("C" if i == 0 else "c", l)
    out, _ = output_for(cmd)
    out = [x for x in out if x.strip()] or [pick(["(No content)", "Running…", "Error: Exit code 1"])]
    shown = out[: R.randrange(1, 6)]
    for i, l in enumerate(shown):
        l = l[: o.w - 7]
        o.add("O", ("  ⎿  " if i == 0 else "     ") + l)
    if len(out) > len(shown):
        o.add("O", f"     … +{len(out) - len(shown)} lines (ctrl+o to expand)")


def cc_tool(o):
    tool = pick(["Read", "Update", "Write", "Search", "Fetch", "Task", "Update Todos", "List", "Glob"])
    if tool == "Search":
        head = f'Search(pattern: "{word()}", path: "{pick(DIRS)}")'
    elif tool == "Fetch":
        head = f"Fetch(https://{host()}/{word()})"
    elif tool == "Task":
        head = f"Task({pick(['Explore the codebase', 'Find the retry logic', 'Review the diff'])})"
    elif tool == "Update Todos":
        head = "Update Todos"
    else:
        head = f"{tool}({path()})"
    o.add("T", f"{cc_bullet()} {head}"[: o.w])
    if tool == "Update":
        p = path()
        o.add("O", f"  ⎿  Updated {p} with {num(1, 9)} additions and {num(1, 9)} removals")
        for _ in range(R.randrange(1, 6)):
            n = num(10, 300)
            o.add("O", f"       {n:>3} {pick([' ', '-', '+'])}  {pick(out_git_diff()[5:])[1:]}"[: o.w])
    elif tool == "Update Todos":
        for _ in range(R.randrange(2, 5)):
            o.add("O", f"  ⎿  {pick(['☐', '☒', '☐'])} {pick(ACTS).capitalize()} {pick(THINGS).replace('{w}', word())}")
    elif tool in ("Read", "Write"):
        o.add("O", f"  ⎿  {'Read' if tool == 'Read' else 'Wrote'} {num(10, 400)} lines {pick(['(ctrl+o to expand)', 'to ' + path()])}")
    else:
        o.add("O", f"  ⎿  {pick(['Found ' + num(1, 9) + ' files (ctrl+o to expand)', 'Received ' + num(1, 90) + 'KB (200 OK)', 'Done (' + num(2, 30) + ' tool uses · 1m 3s)'])}")


def cc_plan(o):
    o.add("P", f"{cc_bullet()} {pick(['Plan:', 'Here is the plan:', 'Next steps:', 'What I changed:', 'The plan:'])}")
    if chance(0.5):
        o.blank()
    for i in range(R.randrange(2, 6)):
        if chance(0.35):
            item = f"{pick(['Run', 'Check with', 'Verify with', 'Rerun'])} `{inline_cmd()}`{pick(['', ' and fix what it reports', ' afterwards'])}"
        elif chance(0.2):
            item = f"{pick(ACTS).capitalize()} {pick(THINGS)} in `{path()}`"
        else:
            item = f"{pick(ACTS).capitalize()} {pick(THINGS)} {pick(['', pick(REASONS)])}".rstrip()
        item = item.replace("{w}", word())
        for line in wrap(item, o.w - 1, f"  {i + 1}. ", "     "):
            o.add("P", line)


def shell_construct(o, indent):
    head, tail = pick([
        (f"for {pick(['f', 'svc', 'host', 'pkg'])} in {' '.join(R.sample(WORDS, 3))}; do", "done"),
        (f"if [ -f {path()} ]; then", "fi"), ("while read -r line; do", f"done < {word()}.txt"),
        (f"{word()}() {{", "}"), (f"until curl -sf localhost:{num(3000, 9000)}/healthz; do", "done")])
    o.add("K", indent + head)
    for _ in range(R.randrange(1, 4)):
        o.add("c", indent + pick(["  ", "    "]) + command().split(" && ")[0])
    o.add("c", indent + tail)


def command_list(o, indent, numbered):
    for i in range(R.randrange(2, 5)):
        marker = f"{i + 1}. " if numbered else pick(["- ", "* ", "• "])
        o.add("K", indent + marker + command().split(" && ")[0])


def code_block(o, indent="  ", fenced=False, prompts=False):
    if fenced:
        o.add("T", indent + pick(["```bash", "```sh", "```", "```shell", "```console"]))
    if not prompts and chance(0.15):
        shell_construct(o, indent)
        if fenced:
            o.add("T", indent + "```")
        return
    for _ in range(R.randrange(1, 4)):
        cmd = command(long=chance(0.25))
        if prompts:
            o.add("C", indent + "$ " + cmd)
            continue
        lines = textwrap.wrap(cmd, width=max(20, o.w - len(indent) - 1), break_long_words=True, break_on_hyphens=False)
        for i, l in enumerate(lines):
            o.add("K" if i == 0 else "c", indent + l)
    if fenced:
        o.add("T", indent + "```")


def cc_answer(o):
    cc_prose(o, paragraph(1).rstrip(".") + pick([":", ":", ". Run:", ". To do that:"]))
    o.blank()
    if chance(0.15):
        command_list(o, "  ", numbered=chance(0.5))
    else:
        code_block(o, indent=pick(["  ", "  ", "    ", "  "]), fenced=chance(0.15), prompts=chance(0.1))
    if chance(0.6):
        o.blank()
        for line in wrap(paragraph(), o.w - 1, "  ", "  "):
            o.add("P", line)


def cc_permission(o):
    w = o.w - 1
    cmd = command(long=chance(0.4))
    inner = w - 4
    o.add("T", "╭" + "─" * (w - 2) + "╮")
    def boxed(label, text):
        o.add(label, "│ " + text.ljust(inner) + " │")
    boxed("T", "Bash command")
    boxed("T", "")
    lines = textwrap.wrap(cmd, width=max(10, inner - 2), break_long_words=True, break_on_hyphens=False)
    for i, l in enumerate(lines):
        boxed("K" if i == 0 else "c", "  " + l)
    boxed("P", "  " + pick(["Clear stale build output and rebuild", "Run the integration tests", "Install the dependencies",
                            "Check the service status", "Push the branch", f"Fetch the {word()} report"]))
    boxed("T", "")
    boxed("T", "Do you want to proceed?")
    boxed("T", "❯ 1. Yes")
    boxed("T", "  2. Yes, and don't ask again for " + cmd.split()[0] + " commands")
    boxed("T", "  3. No, and tell Claude what to do differently (esc)")
    o.add("T", "╰" + "─" * (w - 2) + "╯")


def cc_input(o):
    w = o.w - 1
    typed = pick(["", "", "", paragraph(1)[: w - 8].rsplit(" ", 1)[0], "fix the failing test", "run the tests again"])
    if chance(0.5):
        o.add("T", "╭" + "─" * (w - 2) + "╮")
        o.add("T", "│ > " + typed.ljust(w - 6) + " │")
        o.add("T", "╰" + "─" * (w - 2) + "╯")
    else:
        o.add("T", "─" * w)
        o.add("T", ("> " + typed).rstrip())
        o.add("T", "─" * w)
    status = pick(["  ? for shortcuts", "  ⏵⏵ accept edits on (shift+tab to cycle)", "  ⏸ plan mode on (shift+tab to cycle)", ""])
    if status:
        tail = pick(["", f"Context left until auto-compact: {num(5, 40)}%", "◯ auto-accept off", f"{num(1, 9)} MCP servers failed"])
        if tail and len(status) + len(tail) + 4 < w:
            status = status + " " * (w - len(status) - len(tail)) + tail
        o.add("T", status)


def cc_spinner(o):
    o.add("T", f"{pick(['✻', '✳', '✶', '✢', '·', '✽'])} {pick(['Thinking', 'Clippying', 'Compiling', 'Pondering', 'Running', 'Reticulating'])}… "
               f"({num(1, 90)}s · ↓ {num(1, 9)}.{num(1, 9)}k tokens · esc to interrupt)")


def cc_user(o):
    for line in wrap(pick(["fix the flaky upload test", "why does the build fail on CI?", "run the tests and tell me what broke",
                           "rename the cart module to basket", paragraph(1).lower()]), o.w - 1, pick(["> ", "› ", "> "]), "  "):
        o.add("P", line)


def cc_bang(o):
    cmd = command()
    o.add("C", pick(["! ", "!", "> !"]) + cmd)
    out, _ = output_for(cmd)
    for i, l in enumerate([x for x in out if x.strip()][:4]):
        o.add("O", ("  ⎿  " if i == 0 else "     ") + l[: o.w - 7])


def cc_ran(o):
    cmd = command()
    o.add("C", f"• Ran {cmd}"[: o.w])
    out, _ = output_for(cmd)
    for i, l in enumerate([x for x in out if x.strip()][:4]):
        o.add("O", ("  └ " if i == 0 else "    ") + l[: o.w - 5])


def cc_session(o):
    global BULLETS
    BULLETS = pick([["●", "●", "⏺"], ["●"], ["⏺"], ["•"], ["✦"]])
    steps = R.randrange(3, 10)
    if chance(0.3):
        cc_user(o)
        o.blank()
    for _ in range(steps):
        r = R.random()
        if r < 0.25:
            cc_prose(o)
        elif r < 0.5:
            cc_bash(o)
        elif r < 0.62:
            cc_tool(o)
        elif r < 0.72:
            cc_plan(o)
        elif r < 0.84:
            cc_answer(o)
        elif r < 0.88:
            cc_user(o)
        elif r < 0.92:
            cc_bang(o)
        elif r < 0.95:
            cc_ran(o)
        else:
            cc_spinner(o)
        o.blank()
    r = R.random()
    if r < 0.2:
        cc_permission(o)
    elif r < 0.75:
        if chance(0.4):
            cc_spinner(o)
            o.blank()
        cc_input(o)


# ---------------------------------------------------------------------- docs

def man_session(o):
    name = pick(TOOLS + ["ls", "grep", "tar", "rsync", "git-log"])
    head = f"{name.upper()}(1)"
    mid = pick(["User Commands", "General Commands Manual", "Git Manual"])
    gap = max(1, (o.w - 2 * len(head) - len(mid)) // 2)
    o.add("T", head + " " * gap + mid + " " * gap + head)
    o.blank()
    for section in R.sample(["NAME", "SYNOPSIS", "DESCRIPTION", "OPTIONS", "EXAMPLES", "SEE ALSO", "ENVIRONMENT"], R.randrange(2, 5)):
        o.add("P", section)
        if section == "NAME":
            o.add("P", f"       {name} - {pick(['list directory contents', 'sync files', 'manage ' + word() + 's', 'show commit logs'])}")
        elif section == "SYNOPSIS":
            o.add("P", f"       {name} [OPTION]... [{pick(['FILE', 'PATH', 'SRC', 'COMMIT'])}]...")
        elif section == "OPTIONS":
            for _ in range(R.randrange(1, 4)):
                o.add("P", f"       -{pick('acflrv')}, --{word()}")
                o.add("P", f"              {sentence()}"[: o.w - 1])
        elif section == "EXAMPLES":
            o.add("K", f"       {name} {pick(SUBS)} --{word()} {word()}")
        else:
            for line in wrap(paragraph(), o.w - 8, "       ", "       "):
                o.add("P", line)
        o.blank()
    o.add("T", f" Manual page {name}(1) line {num(1, 90)} (press h for help or q to quit)")


def doc_session(o):
    if chance(0.25):
        man_session(o)
        return
    indent = pick(["", "", "  "])
    w = o.w - len(indent) - 1
    for _ in range(R.randrange(2, 7)):
        r = R.random()
        if r < 0.15:
            o.add("P", indent + pick([f"# {word().capitalize()} setup", f"## {pick(['Install', 'Usage', 'Configuration', 'Troubleshooting'])}",
                                      pick(["GETTING STARTED", "INSTALLATION", "USAGE"]), f"{word().capitalize()} {word()}"]))
        elif r < 0.5:
            for line in wrap(paragraph(R.randrange(1, 4)), w, indent, indent):
                o.add("P", line)
        elif r < 0.6:
            code_block(o, indent=indent + pick(["    ", "  ", "    "]), fenced=chance(0.4), prompts=chance(0.3))
        elif r < 0.66:
            command_list(o, indent + pick(["", "  "]), numbered=chance(0.6))
        elif r < 0.75:
            # A console block: prompts with output.
            o.add("T", indent + "```console") if chance(0.5) else None
            for _ in range(R.randrange(1, 3)):
                cmd = command()
                o.add("C", indent + "$ " + cmd)
                for l in [x for x in output_for(cmd)[0] if x.strip()][:3]:
                    o.add("K" if isinstance(l, Cmd) else "O", (indent + l)[: o.w])
        elif r < 0.9:
            for _ in range(R.randrange(2, 5)):
                for line in wrap(sentence(), w, indent + pick(["• ", "- ", "* "]), indent + "  "):
                    o.add("P", line)
        else:
            for i in range(R.randrange(2, 5)):
                for line in wrap(sentence(), w, indent + f"{i + 1}. ", indent + "   "):
                    o.add("P", line)
        o.blank()
    if chance(0.7):
        o.add("T", pick([f"{word().upper()}.md lines 1-{num(10, 40)}/{num(100, 400)} {num(1, 99)}%", ":", "(END)",
                         f"Manual page {pick(TOOLS)}(1) line 1 (press h for help or q to quit)", f"README.md  {num(1, 99)}%"]))


# ----------------------------------------------------------------------- TUIs

def editor_session(o):
    rows = pick([out_cat_script(), out_cat_config(), out_cat_dockerfile(), out_cat_workflow(),
                 [f"{pick(['pick', 'squash', 'fixup', 'reword'])} {hexhash()} {pick(MSGS)}" for _ in range(R.randrange(2, 6))] + [""]
                 + [comment() or "#" for _ in range(R.randrange(2, 6))]])
    for r in rows:
        if r:
            o.add("K" if isinstance(r, Cmd) else "O", r[: o.w])
        else:
            o.blank()
    for _ in range(R.randrange(1, 8)):
        o.add("T", "~")
    o.add("T", pick([f'"{path()}" {num(5, 90)}L, {num(100, 4000)}B', "-- INSERT --", f"{path()} [+]" + " " * 20 + f"{num(1, 90)},{num(1, 40)}  All",
                     f"  GNU nano 7.2    {path()}  Modified"]))


def watch_session(o):
    cmd = command().split(" && ")[0]
    left = f"Every {pick(['2.0', '1.0', '5.0'])}s: {cmd}"
    right = f"{pick(HOSTS)}: Wed Nov 12 {clock()} 2026"
    o.add("T", left + " " * max(1, o.w - len(left) - len(right) - 1) + right if len(left) + len(right) + 2 < o.w else left)
    o.blank()
    o.output(output_for(cmd)[0][:12] or out_kubectl_get())


def tui_session(o):
    r = R.random()
    if r < 0.3:
        editor_session(o)
        return
    if r < 0.5:
        watch_session(o)
        return
    if r < 0.6:
        for i in range(R.randrange(2, 5)):
            o.add("O", f"    {i}[{'|' * R.randrange(1, 20):<22}{num(1, 99)}.{num(1, 9)}%]   "
                       + pick(['Tasks: 112, 431 thr; 2 running', 'Load average: 0.52 0.61 0.58', 'Uptime: 3 days, 04:11:52']))
        o.blank()
        o.add("O", "    PID USER      PRI  NI  VIRT   RES   SHR S CPU% MEM%   TIME+  Command")
        for _ in range(R.randrange(2, 8)):
            o.add("O", (f"   {num(100, 9999):>4} {pick(USERS):<9}  20   0 {num(10, 900)}M  {num(1, 300)}M  {num(1, 90)}M S {num(0, 40)}.{num(0, 9)}  "
                        f"{num(0, 9)}.{num(0, 9)}  0:{num(10, 59)}.{num(10, 99)} {command().split(' && ')[0]}")[: o.w])
        o.add("T", "F1Help  F2Setup F3SearchF4FilterF5Tree  F6SortByF7Nice -F8Nice +F9Kill  F10Quit"[: o.w])
        return
    w = o.w - 1
    half = w // 2
    title_l, title_r = pick([("Branches", "Commits"), ("Files", "Diff"), ("Pods", "Logs"), ("Processes", "Details")])
    o.add("T", f"┌─ {title_l} " + "─" * (half - len(title_l) - 5) + "┐" + f"┌─ {title_r} " + "─" * (w - half - len(title_r) - 5) + "┐")
    for _ in range(R.randrange(3, 12)):
        left = pick([f"{pick(['*', ' '])} {pick(BRANCHES)}", f" M {path()}", f"{proj()}-{hexhash(5)}  Running", f"{num(100, 9999)} {pick(SERVICES)}"])
        right = pick([f"{hexhash()} {pick(MSGS)}", f"{stamp()} INFO {word()} ok", f"+ {word()} = {num()}", f"CPU {num(1, 99)}%"])
        o.add("O", "│ " + left[: half - 4].ljust(half - 4) + " ││ " + right[: w - half - 5].ljust(w - half - 5) + " │")
    o.add("T", "└" + "─" * (half - 2) + "┘└" + "─" * (w - half - 2) + "┘")
    o.add("T", " " + pick(["p: pull  P: push  c: commit  b: branches  ?: help", "q quit  / filter  enter open", "<esc> back  <ctrl-d> delete  <l> logs"]))


# -------------------------------------------------------------------- screens

WIDTHS = [40, 48, 60, 72, 80, 80, 80, 90, 100, 100, 110, 120, 120, 132, 150, 180, 200]


def make_screen(i):
    w = pick(WIDTHS)
    o = Out(w)
    r = R.random()
    if r < 0.40:
        kind = "shell"
        shell_session(o)
    elif r < 0.76:
        kind = "cc"
        cc_session(o)
    elif r < 0.87:
        kind = "doc"
        doc_session(o)
    elif r < 0.92:
        kind = "mixed"
        shell_session(o)
        o.blank()
        cc_session(o)
    else:
        kind = "tui"
        tui_session(o)
        if chance(0.5):
            shell_session(o)
    rows = o.rows
    # Clip rows that ended up wider than the pane (box art at tiny widths).
    rows = [(lab, text[:w], soft) for lab, text, soft in rows]
    h = R.randrange(8, 61)
    if len(rows) > h:
        start = len(rows) - h if chance(0.7) else R.randrange(0, len(rows) - h + 1)
        rows = rows[start:start + h]
    if chance(0.1):
        rows = rows + [("B", "", False)] * R.randrange(1, 8)
    return f"syn-{i:05d}-{kind}", w, rows


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--screens", type=int, default=6000)
    ap.add_argument("--seed", type=int, default=20261112)
    args = ap.parse_args()
    R.seed(args.seed)
    out = sys.stdout
    out.write(f"# Synthetic screens from tools/generate.py --seed {args.seed} --screens {args.screens}\n\n")
    for i in range(args.screens):
        sid, w, rows = make_screen(i)
        out.write(f"=== {sid} cols={w} | synthetic\n")
        for lab, text, soft in rows:
            if lab == "B" or not text.strip():
                lab = "B" if not text.strip() else lab
            tag = lab + ("~" if soft else "")
            out.write(f"{tag}| {text}\n" if text else f"{tag}|\n")
        out.write("\n")


if __name__ == "__main__":
    main()
