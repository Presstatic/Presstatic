import subprocess, re, json, sys, time
out = open(sys.argv[1], "a")
def ms(v, u): return float(v) * {"us": 0.001, "ms": 1, "s": 1000}[u]
def run(label, port, host, enc, c, dur, path="/", script=None):
    cmd = ["timeout", "40", "wrk", "-t1", f"-c{c}", f"-d{dur}", "--latency", "-H", f"Host: {host}", "-H", f"Accept-Encoding: {enc}"]
    if script: cmd += ["-s", script]
    r = subprocess.run(cmd + [f"http://127.0.0.1:{port}{path}"], capture_output=True, text=True).stdout
    try:
        rps = float(re.search(r"Requests/sec:\s+([\d.]+)", r).group(1)); avg = ms(*re.search(r"Latency\s+([\d.]+)(us|ms|s)", r).groups()); p99 = ms(*re.search(r"99%\s+([\d.]+)(us|ms|s)", r).groups())
        sock = re.search(r"timeout (\d+)", r); non2 = re.search(r"Non-2xx or 3xx responses: (\d+)", r); tot = int(re.search(r"(\d+) requests in", r).group(1))
        row = dict(label=label, enc=enc, conc=c, path=path if not script else "archivio", rps=rps, avg_ms=avg, p99_ms=p99, timeouts=int(sock.group(1)) if sock else 0, non2xx=int(non2.group(1)) if non2 else 0, requests=tot)
        print(f"{label:46} {enc:9} c={c:<4} {row['path']:16} {rps:>9.0f} rich/s  media {avg:>9.2f} ms  99% {p99:>9.2f} ms  timeout {row['timeouts']:>4}  non-200 {row['non2xx']:>4}  richieste {tot}", flush=True)
    except Exception as e:
        row = dict(label=label, error=str(e), raw=r[-300:]); print("ERRORE", row, flush=True)
    out.write(json.dumps(row) + "\n"); out.flush()
    if "senza cache" in label or "fredde" in label: subprocess.run("bash /tmp/fpm-restart.sh > /dev/null", shell=True)
    subprocess.run(": > /var/log/nginx/access.log", shell=True); time.sleep(2)
WP, PS = "wp.local", "www.presstatic.local"
phase = sys.argv[2]
if phase == "caldo":
    for enc in ("gzip", "br, gzip"):
        for path in ("/", "/articolo-1000/"):
            run("WordPress + Varnish", 8210, WP, enc, 50, "10s", path)
            run("WordPress + Nginx, cache FastCGI", 8202, WP, enc, 50, "10s", path)
            run("Presstatic + Nginx", 8204, PS, enc, 50, "10s", path)
elif phase == "connessioni":
    for enc in ("gzip", "br, gzip"):
        for c in (10, 200):
            run("WordPress + Varnish", 8210, WP, enc, c, "8s")
            run("Presstatic + Nginx", 8204, PS, enc, c, "8s")
elif phase == "freddo":
    subprocess.run("varnishadm -n /tmp/varnish 'ban req.url ~ .' >/dev/null; rm -rf /var/cache/nginx/wp/*; nginx -s reload", shell=True); time.sleep(2)
    run("WordPress + Varnish, pagine fredde", 8210, WP, "br, gzip", 50, "10s", script="/tmp/fredde.lua")
    run("WordPress + Nginx, cache FastCGI, pagine fredde", 8202, WP, "br, gzip", 50, "10s", script="/tmp/fredde.lua")
    run("Presstatic + Nginx, pagine fredde", 8204, PS, "br, gzip", 50, "10s", script="/tmp/fredde.lua")
