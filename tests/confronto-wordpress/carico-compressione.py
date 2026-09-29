import subprocess, re, json, sys, time
TARGETS = [("WordPress + Nginx, senza cache", 8201, "wp.local"), ("WordPress + Nginx con cache di pagina", 8202, "wp.local"),
           ("WordPress + Apache, senza cache", 8203, "wp.local"), ("Presstatic + Nginx", 8204, "www.presstatic.local"),
           ("Presstatic + Apache", 8205, "www.presstatic.local")]
PAGES = {"home": "/", "articolo": "/articolo-1000/"}
MODES = {"spenta": "identity", "gzip": "gzip", "brotli": "br"}
conc = [int(x) for x in sys.argv[1].split(",")]; dur = sys.argv[2]; pages = sys.argv[3].split(","); out = open(sys.argv[4], "a")
only_modes = sys.argv[5].split("|") if len(sys.argv) > 5 else list(MODES)


def ms(v, unit):
    return float(v) * {"us": 0.001, "ms": 1, "s": 1000}[unit]


def clean_logs():
    # a decine di migliaia di richieste al secondo i registri delle visite riempirebbero il disco
    subprocess.run(": > /var/log/nginx/access.log; : > /var/log/apache2/access.log; : > /var/log/apache2/other_vhosts_access.log", shell=True)


for mode in only_modes:
    enc = MODES[mode]
    for c in conc:
        for name, port, host in TARGETS:
            for pname in pages:
                r = subprocess.run(["timeout", "30", "wrk", "-t1", f"-c{c}", f"-d{dur}", "--latency", "-H", f"Host: {host}",
                                    "-H", f"Accept-Encoding: {enc}", f"http://127.0.0.1:{port}{PAGES[pname]}"], capture_output=True, text=True).stdout
                try:
                    rps = float(re.search(r"Requests/sec:\s+([\d.]+)", r).group(1))
                    avg = re.search(r"Latency\s+([\d.]+)(us|ms|s)", r); p99 = re.search(r"99%\s+([\d.]+)(us|ms|s)", r)
                    non2 = re.search(r"Non-2xx or 3xx responses: (\d+)", r)
                    sock = re.search(r"Socket errors: connect (\d+), read (\d+), write (\d+), timeout (\d+)", r)
                    tr = re.search(r"Transfer/sec:\s+([\d.]+\w+)", r).group(1)
                    row = {"mode": mode, "conc": c, "target": name, "page": pname, "rps": rps, "avg_ms": ms(*avg.groups()), "p99_ms": ms(*p99.groups()),
                           "non2xx": int(non2.group(1)) if non2 else 0, "timeouts": int(sock.group(4)) if sock else 0,
                           "errors": sum(int(x) for x in sock.groups()[:3]) if sock else 0, "transfer": tr}
                    print(f"{mode:20} c={c:<4} {name:40} {pname:9} {rps:>9.0f} rich/s  media {row['avg_ms']:>9.2f} ms  99% {row['p99_ms']:>9.2f} ms  "
                          f"timeout {row['timeouts']:>4}  errori {row['non2xx'] + row['errors']:>5}  {tr}/s", flush=True)
                except Exception as e:
                    row = {"mode": mode, "conc": c, "target": name, "page": pname, "error": str(e), "raw": r[-300:]}
                    print("ERRORE", row, flush=True)
                out.write(json.dumps(row) + "\n"); out.flush()
                if "senza cache" in name:
                    subprocess.run("bash /tmp/fpm-restart.sh > /dev/null", shell=True)
                clean_logs()
                time.sleep(2)
