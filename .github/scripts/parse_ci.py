#!/usr/bin/env python3
"""
parse_ci.py — runs on the GHA runner, parses vps_ci_gate.sh output into JSON.

Usage:
    python3 parse_ci.py /tmp/ci_raw.txt /tmp/ci_result.json
"""
import json, re, sys, datetime

def extract_section(text, tag):
    """Extract content between === STEP tag === and === STEP tag exit=N duration=Ns ==="""
    pattern = rf'=== STEP {tag} ===\n(.*?)=== STEP {tag} exit=(\d+) duration=(\d+)s ==='
    m = re.search(pattern, text, re.DOTALL)
    if m:
        return m.group(1).strip(), int(m.group(2)), int(m.group(3))
    return "", -1, 0

def extract_meta(text):
    meta = {}
    m = re.search(r'=== META ===(.*?)=== META END ===', text, re.DOTALL)
    if m:
        for line in m.group(1).strip().splitlines():
            if '=' in line:
                k, _, v = line.partition('=')
                meta[k.strip()] = v.strip()
    return meta

def parse_test_output(out):
    passed  = sum(int(x) for x in re.findall(r'(\d+) passed',  out))
    failed  = sum(int(x) for x in re.findall(r'(\d+) failed',  out))
    ignored = sum(int(x) for x in re.findall(r'(\d+) ignored', out))
    failures = []
    in_fail = False
    for line in out.splitlines():
        if line.strip() == 'failures:':
            in_fail = True
            continue
        if in_fail:
            s = line.strip()
            if s and not s.startswith('----') and s != 'failures:':
                if re.match(r'^[\w:]+$', s):
                    failures.append(s)
            if line.startswith('test result:'):
                in_fail = False
    return passed, failed, ignored, failures[:20]

def parse_clippy_output(out):
    errors = []
    current = {}
    for line in out.splitlines():
        m = re.match(r'^error(\[(\w+)\])?: (.+)', line)
        if m:
            if current:
                errors.append(current)
            current = {'code': m.group(2) or 'E', 'message': m.group(3)[:120]}
        loc = re.match(r'^\s+-->\s+(.+):(\d+):(\d+)', line)
        if loc and current and 'file' not in current:
            current['file'] = loc.group(1)
            current['line'] = int(loc.group(2))
            current['col']  = int(loc.group(3))
    if current:
        errors.append(current)
    return errors[:20]

def parse_audit_output(out):
    vulns, current = [], {}
    for line in out.splitlines():
        if 'ID:' in line:
            if current: vulns.append(current)
            current = {'id': line.split('ID:')[-1].strip()}
        elif 'Crate:'    in line and current: current['crate']    = line.split('Crate:')[-1].strip()
        elif 'Version:'  in line and current: current['version']  = line.split('Version:')[-1].strip()
        elif 'Title:'    in line and current: current['title']    = line.split('Title:')[-1].strip()[:100]
        elif 'Severity:' in line and current: current['severity'] = line.split('Severity:')[-1].strip()
        elif 'URL:'      in line and current: current['url']      = line.split('URL:')[-1].strip()
    if current: vulns.append(current)
    return vulns[:20]

def parse_service_section(text):
    m = re.search(r'=== STEP service ===(.*?)=== STEP service ===', text, re.DOTALL)
    if not m:
        return 'unknown', '', []
    lines = [l for l in m.group(1).strip().splitlines() if l.strip()]
    status    = lines[0] if lines else 'unknown'
    since     = lines[1] if len(lines) > 1 else ''
    log_lines = lines[2:] if len(lines) > 2 else []
    return status, since, log_lines[-5:]

def main():
    raw_path    = sys.argv[1] if len(sys.argv) > 1 else '/tmp/ci_raw.txt'
    output_path = sys.argv[2] if len(sys.argv) > 2 else '/tmp/ci_result.json'

    try:
        raw = open(raw_path).read()
    except FileNotFoundError:
        raw = ""

    if not raw.strip() or '=== DONE ===' not in raw:
        # SSH failed or script didn't complete — write minimal error record
        doc = {
            "schema_version": "1.0",
            "meta": {
                "run_at": datetime.datetime.utcnow().strftime('%Y-%m-%dT%H:%M:%SZ'),
                "commit": "unknown", "commit_full": "unknown",
                "commit_message": "unknown", "branch": "master",
                "overall_status": "ERROR", "triggered_by": "push",
                "error": "SSH connection failed or script did not complete"
            },
            "environment": {},
            "steps": {
                "tests":  {"status":"ERROR","exit_code":-1,"duration_seconds":0,"total_passed":0,"total_failed":0,"total_ignored":0,"failures":[]},
                "clippy": {"status":"ERROR","exit_code":-1,"duration_seconds":0,"error_count":0,"errors":[]},
                "audit":  {"status":"ERROR","exit_code":-1,"duration_seconds":0,"vulnerability_count":0,"vulnerabilities":[]}
            },
            "service": {"name":"hedgebot","status":"unknown","active_since":"","recent_logs":[]},
            "raw_output_tail": raw[-3000:]
        }
        open(output_path, 'w').write(json.dumps(doc, indent=2))
        print(f"ERROR: wrote fallback record to {output_path}")
        sys.exit(1)

    meta = extract_meta(raw)

    test_out,   test_exit,   test_dur   = extract_section(raw, 'tests')
    clippy_out, clippy_exit, clippy_dur = extract_section(raw, 'clippy')
    audit_out,  audit_exit,  audit_dur  = extract_section(raw, 'audit')

    t_passed, t_failed, t_ignored, t_failures = parse_test_output(test_out)
    c_errors  = parse_clippy_output(clippy_out)
    a_vulns   = parse_audit_output(audit_out)
    svc_status, svc_since, svc_logs = parse_service_section(raw)

    overall = "PASSED" if (test_exit == 0 and clippy_exit == 0 and audit_exit == 0) else \
              ("ADVISORY" if (test_exit == 0 and clippy_exit == 0 and audit_exit != 0) else "FAILED")

    doc = {
        "schema_version": "1.0",
        "meta": {
            "run_at":         meta.get("run_at",  datetime.datetime.utcnow().strftime('%Y-%m-%dT%H:%M:%SZ')),
            "commit":         meta.get("commit",  "unknown"),
            "commit_full":    meta.get("commit_full", "unknown"),
            "commit_message": meta.get("commit_msg",  "unknown"),
            "branch":         meta.get("branch",  "master"),
            "overall_status": overall,
            "triggered_by":   "push"
        },
        "environment": {
            "rustc":       meta.get("rustc",       "unknown"),
            "cargo":       meta.get("cargo_ver",   "unknown"),
            "os_kernel":   meta.get("os",          "unknown"),
            "arch":        meta.get("arch",        "unknown"),
            "ram_total":   meta.get("ram",         "unknown"),
            "swap_active": meta.get("swap_active", "false") == "true"
        },
        "steps": {
            "tests": {
                "status":           "PASSED" if test_exit == 0 else "FAILED",
                "exit_code":        test_exit,
                "duration_seconds": test_dur,
                "total_passed":     t_passed,
                "total_failed":     t_failed,
                "total_ignored":    t_ignored,
                "failures":         t_failures
            },
            "clippy": {
                "status":           "PASSED" if clippy_exit == 0 else "FAILED",
                "exit_code":        clippy_exit,
                "duration_seconds": clippy_dur,
                "error_count":      len(c_errors),
                "errors":           c_errors
            },
            "audit": {
                "status":              "PASSED" if audit_exit == 0 else "ADVISORY",
                "exit_code":           audit_exit,
                "duration_seconds":    audit_dur,
                "vulnerability_count": len(a_vulns),
                "vulnerabilities":     a_vulns
            }
        },
        "service": {
            "name":         "hedgebot",
            "status":       svc_status,
            "active_since": svc_since,
            "recent_logs":  svc_logs
        }
    }

    open(output_path, 'w').write(json.dumps(doc, indent=2))
    print(f"overall={overall}  tests={t_passed}p/{t_failed}f  clippy_errors={len(c_errors)}  vulns={len(a_vulns)}")

if __name__ == '__main__':
    main()
