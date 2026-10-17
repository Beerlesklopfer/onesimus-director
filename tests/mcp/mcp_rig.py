#!/usr/bin/env python3
"""Integration run of oconsole's MCP tools (S-36) against a Rust Director.

Starts onesimus-dird with a PRIVATE bareos-fd/bareos-sd (own configs, ports,
file device and working directories under --base) and a throw-away catalog
(new database from the Bareos DDL, dropped at the end), then drives
`oconsole --mcp` over stdio with JSON-RPC: tools/list and every tool at least
once, over BNET and/or gRPC, including a backup, a one-line restore and an
interactive restore (restore_tree_*) into --restore-dir.

Never touches the live daemons or bareos_cppref.

  python3 tests/mcp/mcp_rig.py --director target/debug/onesimus-dird \\
      --oconsole target/debug/oconsole --base /tmp/claude-10001/m36 \\
      --port 39250 --restore-dir <dir> --transports bnet,grpc --report out.json

Ports: --port (Director BNET), +1 FD, +2 SD, +3 gRPC.
"""

import argparse
import hashlib
import json
import os
import select
import shutil
import socket
import subprocess
import sys
import time

PASSWORD = "m36-dir-pw"


def wait_port(port, seconds=30):
    end = time.time() + seconds
    while time.time() < end:
        try:
            socket.create_connection(("127.0.0.1", port), 1).close()
            return True
        except OSError:
            time.sleep(0.3)
    return False


def psql(sql, db="postgres"):
    return subprocess.run(["psql", "-d", db, "-v", "ON_ERROR_STOP=1", "-Atc", sql],
                          capture_output=True, text=True)


class Rig:
    def __init__(self, args):
        self.args = args
        self.base = os.path.abspath(args.base)
        self.port = args.port
        self.fd_port, self.sd_port, self.grpc_port = self.port + 1, self.port + 2, self.port + 3
        self.db = f"mcp_rig_{self.port}_{os.getpid()}"
        self.procs = []

    def path(self, *p):
        return os.path.join(self.base, *p)

    def setup(self):
        shutil.rmtree(self.base, ignore_errors=True)
        for d in ["data", "storage", "dirwork", "fd-work", "sd-work", "fd", "sd", "dir/bareos-dir.d/console",
                  "dir/bareos-dir.d/profile", "dir/bareos-dir.d/user", "dir/bareos-dir.d/counter", "dir/bareos-dir.d/jobdefs"]:
            os.makedirs(self.path(d), exist_ok=True)
        for i in range(3):
            with open(self.path("data", f"file{i}.txt"), "w") as f:
                f.write(f"content {i}\n" * (i + 1))
        with open(self.path("fd", "bareos-fd.conf"), "w") as f:
            f.write(f"Client {{ Name = tfd; FD Port = {self.fd_port}; FD Address = 127.0.0.1; "
                    f"Working Directory = {self.path('fd-work')}; }}\n"
                    "Director { Name = bareos-dir; Password = \"tfd-pw\" }\n"
                    "Messages { Name = Standard; Director = bareos-dir = all, !skipped, !restored }\n")
        with open(self.path("sd", "bareos-sd.conf"), "w") as f:
            f.write(f"Storage {{ Name = tsd; SD Port = {self.sd_port}; SD Address = 127.0.0.1; "
                    f"Working Directory = {self.path('sd-work')}; }}\n"
                    "Director { Name = bareos-dir; Password = \"tsd-pw\" }\n"
                    f"Device {{ Name = FileStorage; Media Type = File; Archive Device = {self.path('storage')}; "
                    "LabelMedia = yes; Random Access = yes;\n AutomaticMount = yes; RemovableMedia = no; AlwaysOpen = no; }\n"
                    "Messages { Name = Standard; Director = bareos-dir = all }\n")
        psql(f'DROP DATABASE IF EXISTS "{self.db}" WITH (FORCE)')
        r = psql(f"CREATE DATABASE \"{self.db}\" ENCODING 'SQL_ASCII' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0")
        assert r.returncode == 0, r.stderr
        r = subprocess.run(["psql", "-q", "-d", self.db, "-v", "ON_ERROR_STOP=1", "-f",
                            "/usr/lib/bareos/scripts/ddl/creates/postgresql.sql"], capture_output=True, text=True)
        assert r.returncode == 0, r.stderr
        user = os.environ.get("USER", "bareos")
        work = self.path("dirwork")
        files = {
            "director/bareos-dir.conf": f"Director {{\n  Name = bareos-dir\n  Password = \"{PASSWORD}\"\n  Messages = Daemon\n"
                f"  Working Directory = {work}\n  DirPort = {self.port}\n  Enable Grpc = yes\n  Grpc Port = {self.grpc_port}\n"
                "  Maximum Concurrent Jobs = 10\n  QueryFile = \"/usr/lib/bareos/scripts/query.sql\"\n}\n",
            "catalog/MyCatalog.conf": f"Catalog {{\n  Name = MyCatalog\n  dbname = \"{self.db}\"\n  dbuser = \"{user}\"\n  dbpassword = \"\"\n}}\n",
            "messages/Daemon.conf": f"Messages {{\n  Name = Daemon\n  append = \"{work}/daemon.log\" = all, !skipped, !saved, !audit\n}}\n",
            "messages/Standard.conf": f"Messages {{\n  Name = Standard\n  append = \"{work}/job.log\" = all, !skipped, !saved, !audit\n"
                "  catalog = all, !skipped, !saved, !audit\n  console = all, !skipped, !saved, !audit\n}\n",
            "client/tfd.conf": f"Client {{\n  Name = tfd\n  Address = 127.0.0.1\n  FD Port = {self.fd_port}\n  Password = \"tfd-pw\"\n}}\n",
            "storage/tsd.conf": f"Storage {{\n  Name = tsd\n  Address = 127.0.0.1\n  SD Port = {self.sd_port}\n  Password = \"tsd-pw\"\n"
                "  Device = FileStorage\n  Media Type = File\n}\n",
            "pool/TA.conf": "Pool {\n  Name = TA\n  Pool Type = Backup\n  Recycle = yes\n  AutoPrune = no\n  Volume Retention = 30 days\n}\n",
            "pool/Scratch.conf": "Pool {\n  Name = Scratch\n  Pool Type = Scratch\n}\n",
            "fileset/TSet.conf": f"FileSet {{\n  Name = TSet\n  Include {{\n    Options {{\n      Signature = MD5\n    }}\n    File = {self.path('data')}\n  }}\n}}\n",
            "schedule/Never.conf": "Schedule {\n  Name = Never\n}\n",
            "job/Bk.conf": "Job {\n  Name = Bk\n  Type = Backup\n  Level = Full\n  Client = tfd\n  FileSet = TSet\n  Storage = tsd\n"
                "  Pool = TA\n  Messages = Standard\n}\n",
            "job/RestoreFiles.conf": f"Job {{\n  Name = RestoreFiles\n  Type = Restore\n  Client = tfd\n  FileSet = TSet\n  Storage = tsd\n"
                f"  Pool = TA\n  Messages = Standard\n  Where = {self.path('default-restore')}\n}}\n",
        }
        for rel, text in files.items():
            p = self.path("dir", "bareos-dir.d", rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w") as f:
                f.write(text)
        with open(self.path("oconsole.conf"), "w") as f:
            f.write(f"Director {{\n  Name = bareos-dir\n  Address = 127.0.0.1\n  DIRport = {self.port}\n"
                    f"  Password = \"{PASSWORD}\"\n  Grpc Port = {self.grpc_port}\n}}\n")

    def spawn(self, cmd, log):
        f = open(self.path(log), "w")
        env = dict(os.environ, LC_ALL="C", LANG="C", TZ="UTC")
        p = subprocess.Popen(cmd, stdout=f, stderr=subprocess.STDOUT, env=env)
        self.procs.append(p)
        return p

    def start(self):
        self.spawn(["/usr/sbin/bareos-fd", "-f", "-c", self.path("fd", "bareos-fd.conf")], "fd.out")
        self.spawn(["/usr/sbin/bareos-sd", "-f", "-c", self.path("sd", "bareos-sd.conf")], "sd.out")
        assert wait_port(self.fd_port) and wait_port(self.sd_port), "private daemons did not start"
        self.spawn([self.args.director, "-f", "-c", self.path("dir", "bareos-dir.d")], "dir.out")
        assert wait_port(self.port) and wait_port(self.grpc_port), "Director did not open its ports"
        cert = self.path("dirwork", "onesimus-grpc.crt")
        der = subprocess.run(["openssl", "x509", "-in", cert, "-outform", "der"], capture_output=True).stdout
        self.fingerprint = hashlib.sha256(der).hexdigest()

    def stop(self):
        for p in reversed(self.procs):
            if p.poll() is None:
                p.terminate()
        for p in self.procs:
            try:
                p.wait(timeout=15)
            except subprocess.TimeoutExpired:
                p.kill()
        psql(f'DROP DATABASE IF EXISTS "{self.db}" WITH (FORCE)')


class Mcp:
    def __init__(self, cmd, log):
        self.log = open(log, "w")
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
                                  text=True, bufsize=1)
        self.id = 0

    def request(self, method, params, timeout=600):
        self.id += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        end = time.time() + timeout
        while time.time() < end:
            r, _, _ = select.select([self.p.stdout], [], [], 1)
            if not r:
                if self.p.poll() is not None:
                    raise RuntimeError("oconsole exited")
                continue
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("oconsole closed stdout")
            msg = json.loads(line)
            if msg.get("id") == self.id:
                return msg
        raise TimeoutError(method)

    def notify(self, method):
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method}) + "\n")
        self.p.stdin.flush()

    def close(self):
        self.p.stdin.close()
        try:
            self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.p.kill()


def text_of(res):
    return "".join(c.get("text", "") for c in res.get("content", []))


def jobids_of(res):
    sc = res.get("structuredContent") or {}
    ids = sc.get("jobids") or []
    if not ids:
        for j in sc.get("jobs") or []:
            if j.get("jobid"):
                ids.append(j["jobid"])
    if not ids:
        t = text_of(res)
        for part in t.split("JobId=")[1:]:
            digits = "".join(c for c in part[:12] if c.isdigit())
            if digits:
                ids.append(int(digits))
                break
    return [int(i) for i in ids]


def session(rig, transport, restore_dir, report):
    tag = "G" if transport == "grpc" else "B"
    cmd = [rig.args.oconsole, "-c", rig.path("oconsole.conf"), "--mcp"]
    if transport == "grpc":
        cmd += ["--grpc", "--grpc-fingerprint", rig.fingerprint]
    mcp = Mcp(cmd, rig.path(f"oconsole-{transport}.err"))
    init = mcp.request("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                                      "clientInfo": {"name": "mcp-rig", "version": "1"}})
    assert init["result"]["serverInfo"]["name"] == "oconsole", init
    mcp.notify("notifications/initialized")
    tools = [t["name"] for t in mcp.request("tools/list", {})["result"]["tools"]]
    called = {}
    results = []

    def call(name, args, expect_ok=True, note=""):
        msg = mcp.request("tools/call", {"name": name, "arguments": args})
        if "error" in msg:
            res = {"isError": True, "content": [{"type": "text", "text": "JSON-RPC error: " + msg["error"]["message"]}]}
        else:
            res = msg["result"]
        ok = not res.get("isError", False)
        called[name] = called.get(name, 0) + 1
        typed = "structuredContent" in res and transport == "grpc"
        results.append({"transport": transport, "tool": name, "args": args, "ok": ok, "expected_ok": expect_ok,
                        "typed": typed, "note": note, "text": text_of(res)[:600],
                        "structured": res.get("structuredContent")})
        flag = "ok " if ok else ("ERR" if expect_ok else "err")
        print(f"[{transport}] {flag} {name} {json.dumps(args)[:90]}{' (' + note + ')' if note else ''}", flush=True)
        return res

    data = rig.path("data")
    vol = f"M36{tag}-0001"
    call("label_volume", {"storage": "tsd", "volume": vol, "pool": "TA"})
    run = call("run_job", {"job": "Bk", "level": "Full"})
    backup = jobids_of(run)[0]
    call("wait_job", {"jobid": backup})
    # ---- read tools
    reads = [
        ("query", {"command": "list jobs"}), ("version", {}), ("time", {}), ("whoami", {}), ("memory", {}),
        ("dot_noop", {}), ("help", {"command": "list"}), ("dot_help", {"item": "list"}), ("messages", {}),
        ("dot_messages", {}), ("var_expand", {"text": "abc"}), ("resolve", {"hostname": "localhost"}),
        ("estimate", {"job": "Bk", "level": "Full"}),
        ("list_jobs", {"client": "tfd", "long": True}), ("list_job", {"jobid": backup}),
        ("list_joblog", {"jobid": backup}), ("list_jobmedia", {"jobid": backup}), ("list_files", {"jobid": backup}),
        ("list_volumes", {}), ("list_volumes", {"volume": vol}), ("list_volumes", {"media": True}),
        ("list_volumes", {"mediaid": 1}), ("list_pools", {}), ("list_pools", {"poolid": 1}), ("list_clients", {}),
        ("list_storages", {}), ("list_filesets", {}), ("list_jobtotals", {}), ("list_copies", {}),
        ("list_backups", {"client": "tfd"}), ("list_basefiles", {"jobid": backup}),
        ("list_jobstatistics", {"jobid": backup}), ("list_log", {"limit": 5}),
        ("show", {"resource": "job", "name": "Bk"}), ("show", {"resource": "pool"}), ("show", {"resource": "all"}),
        ("show", {"resource": "disabled"}),
        ("status_director", {}), ("status_all", {}), ("status_client", {"client": "tfd"}),
        ("status_storage", {"storage": "tsd"}), ("status_scheduler", {"days": 2}), ("status_subscriptions", {}),
        ("status_configuration", {}), ("status_job", {"job": "Bk"}),
        ("dot_status", {"target": "dir", "keyword": "running"}),
        ("dot_status", {"target": "client", "name": "tfd", "keyword": "header"}),
        ("dot_status", {"target": "storage", "name": "tsd", "keyword": "devices"}),
        ("dot_jobs", {}), ("dot_clients", {}), ("dot_pools", {}), ("dot_storages", {}), ("dot_filesets", {}),
        ("dot_schedule", {}), ("dot_levels", {}), ("dot_types", {}), ("dot_msgs", {}), ("dot_jobstatus", {}),
        ("dot_defaults", {"name": "Bk"}), ("dot_authorized", {"job": "Bk"}),
    ] + [("dot_list", {"kind": k}) for k in ["catalogs", "consoles", "users", "jobdefs", "profiles", "locations",
                                              "media", "mediatypes", "volstatus", "actiononpurge"]] + [
        ("sql_meta", {"kind": "tables"}), ("sql_meta", {"kind": "count", "table": "job"}),
        ("sql_meta", {"kind": "schema", "table": "job"}), ("acl_list", {}), ("profile_list", {}),
        ("ovfs_status", {}), ("ovfs_latest", {"client": "tfd", "fileset": "TSet"}),
        ("ovfs_tree", {"jobids": str(backup), "depth": 1}), ("ovfs_ls", {"jobids": str(backup), "path": data + "/"}),
        ("ovfs_versions", {"client": "tfd", "path": data + "/", "name": "file0.txt"}),
    ]
    for name, args in reads:
        call(name, args)
    # job Bk has no Schedule: both Directors answer "Could not find next Volume for Job Bk."
    # (ListNextvol only looks at the schedule's runs; S-41, tests/parity/cases/s41.jsonl)
    call("list_nextvolume", {"job": "Bk"}, transport == "bnet", "no appendable volume found by the Director")
    # BVFS
    call("bvfs_update", {"jobids": [backup]})
    call("bvfs_get_jobids", {"jobid": backup})
    call("bvfs_lsdirs", {"jobids": [backup], "path": "/"})
    call("bvfs_lsfiles", {"jobids": [backup], "path": data + "/"})
    call("bvfs_versions", {"client": "tfd", "pathid": 1, "filename": "file0.txt"})
    call("bvfs_restore", {"path": f"b2{backup}", "jobid": str(backup), "fileid": "1"}, False, "fileid may not exist")
    call("bvfs_cleanup", {"path": f"b2{backup}"})
    call("bvfs_clear_cache", {})
    # ---- restore in one line
    target_a = os.path.join(restore_dir, transport, "a")
    r = call("restore", {"client": "tfd", "jobids": [backup], "all": True, "where": target_a})
    rid = jobids_of(r)
    if rid:
        call("wait_job", {"jobid": rid[0]})
    # ---- interactive restore
    target_b = os.path.join(restore_dir, transport, "b")
    st = call("restore_tree_start", {"client": "tfd", "jobids": [backup], "where": target_b},
              expect_ok=not (transport == "grpc" and rig.args.production),
              note="production Director over gRPC: no tree RPCs" if transport == "grpc" and rig.args.production else "")
    sid = (st.get("structuredContent") or {}).get("session_id")
    tree_ok = sid is not None
    if tree_ok:
        s = lambda extra=None: dict({"session_id": sid}, **(extra or {}))
        call("restore_tree_cd", s({"path": data + "/"}))
        call("restore_tree_pwd", s())
        call("restore_tree_ls", s())
        call("restore_tree_ls", s({"mode": "dirs"}))
        call("restore_tree_mark", s({"pattern": "file1.txt"}))
        call("restore_tree_mark", s({"pattern": "file2.txt"}))
        call("restore_tree_unmark", s({"pattern": "file2.txt"}))
        call("restore_tree_ls", s({"mode": "marked"}))
        call("restore_tree_count", s())
        call("restore_tree_find", s({"pattern": "file0*"}))
        call("restore_tree_estimate", s())
        call("restore_tree_help", s())
        call("restore_tree_mark", s({"pattern": "..", "dirs_only": True}), False, "markdir on ..")
        call("restore_tree_unmark", s({"pattern": "..", "dirs_only": True}), False, "unmarkdir on ..")
        done = call("restore_tree_done", s())
        did = jobids_of(done)
        if did:
            call("wait_job", {"jobid": did[0]})
        st2 = call("restore_tree_start", {"client": "tfd", "jobids": [backup]})
        sid2 = (st2.get("structuredContent") or {}).get("session_id")
        if sid2:
            call("restore_tree_abort", {"session_id": sid2})
    else:
        for t in ["restore_tree_cd", "restore_tree_pwd", "restore_tree_ls", "restore_tree_mark", "restore_tree_unmark",
                  "restore_tree_count", "restore_tree_find", "restore_tree_estimate", "restore_tree_help",
                  "restore_tree_done", "restore_tree_abort"]:
            args = {"session_id": "none"}
            if t in ("restore_tree_cd",):
                args["path"] = "/"
            if t in ("restore_tree_mark", "restore_tree_unmark", "restore_tree_find"):
                args["pattern"] = "*"
            call(t, args, False, "no session")
    # ---- further write tools
    later = call("run_job", {"job": "Bk", "when": "2099-01-01 00:00:00"})
    lid = jobids_of(later)
    if lid:
        call("cancel_job", {"jobid": lid[0]})
        call("update_job", {"jobid": backup, "client": "tfd"})
        rr = call("rerun_job", {"jobids": [lid[0]]})
        for j in jobids_of(rr):
            call("wait_job", {"jobid": j})
    for t in ["unmount_storage", "mount_storage", "release_storage"]:
        call(t, {"storage": "tsd"})
    call("update_volume", {"volume": vol, "volstatus": "Used"})
    call("update_pool", {"pool": "TA"})
    call("update_slots", {"storage": "tsd"}, False, "not an autochanger")
    call("update_stats", {"days": 1})
    call("create_pool", {"pool": "TA"}, False, "pool exists: the typed RPC reports failure")
    dialog_ok = transport == "bnet" or not rig.args.production
    call("add_volumes", {"pool": "TA", "storage": "tsd", "count": 0, "name": f"M36{tag}-add"},
         expect_ok=transport == "bnet", note="dialog: BNET only (AddVolumes has no fields for the answers)")
    call("export_volumes", {"storage": "tsd", "srcslots": "1"}, False, "not an autochanger")
    call("import_volumes", {"storage": "tsd"}, False, "not an autochanger")
    call("move_volumes", {"storage": "tsd", "srcslots": "1", "dstslots": "2"}, False, "not an autochanger")
    call("enable", {"resource": "job", "name": "Bk"})
    call("disable", {"resource": "schedule", "name": "Never"})
    call("enable", {"resource": "schedule", "name": "Never"})
    call("setdebug", {"level": 0, "target": "dir"})
    call("setbandwidth", {"limit": 0, "client": "tfd"})
    call("setdevice", {"storage": "tsd", "device": "FileStorage", "autoselect": True})
    call("session_option", {"option": "autodisplay", "on": False})
    call("session_option", {"option": "automount", "on": True})
    call("session_option", {"option": "trace", "on": False})
    call("use_catalog", {"catalog": "MyCatalog"})
    call("dot_sql", {"query": "SELECT count(*) AS n FROM job"})
    call("sqlquery", {"query": "SELECT 42 AS answer"}, dialog_ok, "" if dialog_ok else "dialog: not over ExecuteCommand")
    call("run_query", {"number": 1, "parameters": ["tfd"]}, dialog_ok, "" if dialog_ok else "dialog: not over ExecuteCommand")
    cl = f"m36{tag.lower()}-client"
    call("configure_add", {"resource": "client", "name": cl, "directives": {"address": "127.0.0.9", "password": "x"}})
    call("configure_update", {"resource": "client", "name": cl, "directives": {"address": "127.0.0.10"}})
    call("configure_export", {"client": "tfd"})
    con = f"m36{tag.lower()}-console"
    call("configure_add", {"resource": "console", "name": con, "directives": {"password": "x"}})
    call("acl_modify", {"action": "set", "console": con, "type": "job", "value": "Bk"})
    call("acl_list", {"console": con})
    call("acl_modify", {"action": "delete", "console": con, "type": "job", "value": "Bk"})
    call("acl_modify", {"action": "copy", "from": con, "to": con}, False, "copy onto itself")
    call("acl_modify", {"action": "clear", "console": con})
    call("profile_modify", {"action": "create", "name": "m36p"}, False, ".profile create unknown to the Director")
    call("configure_delete", {"resource": "client", "name": cl, "confirm": True})
    call("reload", {})
    # ---- prune / purge / truncate / relabel / delete at the end
    call("prune", {"kind": "files", "client": "tfd", "confirm": True})
    call("prune", {"kind": "jobs", "client": "tfd", "confirm": True})
    call("prune", {"kind": "stats", "confirm": True})
    call("prune", {"kind": "volume", "volume": vol, "confirm": True}, False, "volume still has jobs: typed RPC reports failure")
    call("prune", {"kind": "directory", "directory": data + "/", "client": "tfd", "confirm": True}, False, "may report nothing")
    call("purge", {"kind": "quota", "client": "tfd", "confirm": True})
    call("purge", {"kind": "files", "jobid": backup, "confirm": True})
    call("purge", {"kind": "jobs", "client": "tfd", "confirm": True})
    call("purge", {"kind": "volume", "volume": [vol], "confirm": True})
    call("truncate", {"volume": vol, "storage": "tsd", "pool": "TA", "confirm": True})
    call("relabel_volume", {"storage": "tsd", "oldvolume": vol, "volume": vol + "r", "pool": "TA", "confirm": True})
    call("delete", {"kind": "volume", "volume": vol + "r", "confirm": True})
    if lid:
        call("delete", {"kind": "jobid", "jobids": [lid[0]], "confirm": True})
    call("delete", {"kind": "pool", "pool": "Scratch", "confirm": True}, False, "asks a question")
    call("delete", {"kind": "storage", "storage": "nosuch", "confirm": True}, False, "unknown storage")
    mcp.close()
    report["sessions"].append({"transport": transport, "tools_listed": len(tools),
                               "not_called": sorted(set(tools) - set(called) - {"execute"}),
                               "results": results})
    # restored files
    tree = transport == "bnet" or not rig.args.production
    for sub, want in (("a", [0, 1, 2]), ("b", [1] if tree else [])):
        base = os.path.join(restore_dir, transport, sub) + data
        for i in range(3):
            p = os.path.join(base, f"file{i}.txt")
            exists = os.path.exists(p)
            content_ok = exists and open(p).read() == f"content {i}\n" * (i + 1)
            report["restores"].append({"transport": transport, "target": sub, "file": p,
                                       "expected": i in want, "exists": exists, "content_ok": content_ok})


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--director", required=True)
    ap.add_argument("--oconsole", required=True)
    ap.add_argument("--base", required=True)
    ap.add_argument("--port", type=int, default=39250)
    ap.add_argument("--restore-dir", required=True)
    ap.add_argument("--transports", default="bnet,grpc")
    ap.add_argument("--production", action="store_true", help="the Director is a production build")
    ap.add_argument("--report", required=True)
    args = ap.parse_args()
    rig = Rig(args)
    report = {"director": args.director, "oconsole": args.oconsole, "sessions": [], "restores": []}
    shutil.rmtree(args.restore_dir, ignore_errors=True)
    try:
        rig.setup()
        rig.start()
        for t in args.transports.split(","):
            session(rig, t, args.restore_dir, report)
    finally:
        rig.stop()
        with open(args.report, "w") as f:
            json.dump(report, f, indent=1)
    bad = [r for s in report["sessions"] for r in s["results"] if r["expected_ok"] and not r["ok"]]
    restore_bad = [r for r in report["restores"]
                   if r["expected"] != r["exists"] or (r["expected"] and not r["content_ok"])]
    for s in report["sessions"]:
        print(f"{s['transport']}: {len(s['results'])} calls, {s['tools_listed']} tools listed, not called: {s['not_called']}")
    print(f"unexpected tool errors: {len(bad)}")
    for r in bad:
        print(f"  [{r['transport']}] {r['tool']} {json.dumps(r['args'])[:80]}: {r['text'][:300]!r}")
    print(f"restore mismatches: {len(restore_bad)}")
    for r in restore_bad:
        print(f"  {r}")
    sys.exit(1 if bad or restore_bad else 0)


if __name__ == "__main__":
    main()
