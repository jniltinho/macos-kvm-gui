package mcp

import (
	"context"
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

func TestSSHArgs(t *testing.T) {
	got := sshArgs(2223, "nilton", "/cfg/known_hosts", true, "sw_vers -productVersion")
	want := []string{"-p", "2223", "-o", "ConnectTimeout=10", "-o", "BatchMode=no", "-o", "StrictHostKeyChecking=accept-new",
		"-o", "UserKnownHostsFile=/cfg/known_hosts", "-o", "LogLevel=ERROR", "-o", "NumberOfPasswordPrompts=1",
		"nilton@127.0.0.1", "--", "sw_vers -productVersion"}
	if !slices.Equal(got, want) {
		t.Fatalf("args:\n%q\nwant\n%q", got, want)
	}
	if got := sshArgs(2222, "u", "/k", false, "id"); !slices.Contains(got, "BatchMode=yes") {
		t.Fatalf("without a password ssh must never prompt: %q", got)
	}
}

func TestVMSSHGuards(t *testing.T) {
	cs := session(t)
	e := createVM(t, "Nilton", 2222)
	mustFail(t, cs, "vm_ssh", map[string]any{"vm": "Nilton", "user": "nilton", "command": "id"}, "runs commands in the guest")
	mustFail(t, cs, "vm_ssh", map[string]any{"vm": "Nilton", "user": "nilton", "command": "id", "confirm": true}, "is not running")
	mustFail(t, cs, "vm_ssh", map[string]any{"vm": "Nilton", "user": "-oProxyCommand=x", "command": "id", "confirm": true}, "user:")
	mustFail(t, cs, "vm_ssh", map[string]any{"vm": "Nilton", "user": "nilton", "command": "id", "confirm": true, "timeout_sec": 601}, "timeout_sec")
	// A running VM without a forward.
	old := findRunning
	findRunning = func(core.Config) (core.RunInfo, bool) { return core.RunInfo{PID: 1}, true }
	t.Cleanup(func() { findRunning = old })
	mustFail(t, cs, "vm_ssh", map[string]any{"vm": e.ID, "user": "nilton", "command": "id", "confirm": true}, "no SSH forward")
}

// The askpass socket answers only a child of the ssh it serves, and only once.
func TestAskpassSocket(t *testing.T) {
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	ap, err := newAskpass("s3cret")
	if err != nil {
		t.Fatal(err)
	}
	defer ap.close()
	if st, err := os.Stat(ap.dir); err != nil || st.Mode().Perm() != 0o700 {
		t.Fatalf("the socket's folder must be private: %v %v", st.Mode(), err)
	}
	go ap.serve(os.Getpid()) // this test process plays ssh

	// This process is not a child of itself: no answer.
	c, err := net.Dial("unix", ap.sock)
	if err != nil {
		t.Fatal(err)
	}
	b, _ := io.ReadAll(c)
	c.Close()
	if len(b) != 0 {
		t.Fatalf("a stranger got %q", b)
	}

	helper := func() (string, error) {
		cmd := exec.Command(ap.helper, "Password:") // what ssh runs: the symlink, with its prompt
		cmd.Env = append(os.Environ(), askpassEnv+"="+ap.sock)
		out, err := cmd.Output()
		return string(out), err
	}
	if out, err := helper(); err != nil || out != "s3cret\n" {
		t.Fatalf("the child must get the password: %q %v", out, err)
	}
	if out, err := helper(); err == nil || strings.Contains(out, "s3cret") {
		t.Fatalf("a second ask must fail: %q %v", out, err)
	}
	if _, err := os.Stat(ap.dir); !os.IsNotExist(err) {
		t.Fatal("the socket's folder must be removed after the answer")
	}
}

func TestCappedOutput(t *testing.T) {
	var c capped
	c.Write(make([]byte, outputCap-1))
	c.Write([]byte("abc"))
	if c.buf.Len() != outputCap || !c.truncated || !strings.HasSuffix(c.String(), "[truncated at 1 MiB]") {
		t.Fatalf("len %d truncated %v", c.buf.Len(), c.truncated)
	}
}

// The timeout kills ssh's whole process group.
func TestRunSSHTimeout(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(dir+"/ssh", []byte("#!/bin/sh\nsleep 30 &\necho started\nwait\n"), 0o755)
	t.Setenv("PATH", dir+":"+os.Getenv("PATH"))
	start := time.Now()
	out, err := runSSH(context.Background(), nil, "", time.Second)
	if err != nil || !out.TimedOut || !strings.Contains(out.Stdout, "started") || time.Since(start) > 10*time.Second {
		t.Fatalf("timeout: %+v %v after %s", out, err, time.Since(start))
	}
}

// The real chain, ssh → ssh-askpass → socket, against a throwaway sshd in a container
// (MKG_TEST_SSHD=1; needs Docker and the alpine image).
func TestVMSSHAgainstSSHD(t *testing.T) {
	if os.Getenv("MKG_TEST_SSHD") == "" {
		t.Skip("set MKG_TEST_SSHD=1 to run against an sshd in Docker")
	}
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := l.Addr().(*net.TCPAddr).Port
	l.Close()
	name := fmt.Sprintf("mkg-sshd-%d", port)
	pw := fmt.Sprintf("pw-%d", time.Now().UnixNano())
	script := "apk add --no-cache openssh >/dev/null && ssh-keygen -A >/dev/null && adduser -D tester && " +
		"echo \"tester:$PW\" | chpasswd && exec /usr/sbin/sshd -D -e -o PasswordAuthentication=yes"
	run := exec.Command("docker", "run", "-d", "--rm", "--name", name, "-e", "PW", "-p", fmt.Sprintf("127.0.0.1:%d:22", port), "alpine:latest", "sh", "-c", script)
	run.Env = append(os.Environ(), "PW="+pw) // passed by name: not in docker's argv either
	if out, err := run.CombinedOutput(); err != nil {
		t.Fatalf("docker run: %v %s", err, out)
	}
	t.Cleanup(func() { exec.Command("docker", "rm", "-f", name).Run() })

	cs := session(t)
	t.Setenv("XDG_RUNTIME_DIR", t.TempDir())
	t.Setenv("SSH_AUTH_SOCK", "") // only the password logs in
	t.Setenv("HOME", t.TempDir()) // no user keys or ssh config
	e := createVM(t, "Nilton", 0)
	old := findRunning
	findRunning = func(core.Config) (core.RunInfo, bool) { return core.RunInfo{PID: 1, SSHPort: port}, true }
	t.Cleanup(func() { findRunning = old })

	args := map[string]any{"vm": e.ID, "user": "tester", "command": "echo hello; echo oops >&2; exit 3", "password": pw, "confirm": true}
	var got map[string]any
	for deadline := time.Now().Add(90 * time.Second); ; { // sshd is being installed
		got = callOut(t, cs, "vm_ssh", args)
		if got["exit_code"] != 255.0 || time.Now().After(deadline) {
			break
		}
		time.Sleep(2 * time.Second)
	}
	if got["stdout"] != "hello\n" || got["stderr"] != "oops\n" || got["exit_code"] != 3.0 {
		t.Fatalf("vm_ssh: %v", got)
	}
	// A wrong password fails without hanging, and nothing is left in the runtime folder.
	args["password"] = "wrong"
	if out := callOut(t, cs, "vm_ssh", args); out["exit_code"] != 255.0 {
		t.Fatalf("wrong password: %v", out)
	}
	// Without a password and without keys, BatchMode fails at once instead of prompting.
	delete(args, "password")
	if out := callOut(t, cs, "vm_ssh", args); out["exit_code"] != 255.0 || !strings.Contains(out["stderr"].(string), "Permission denied") {
		t.Fatalf("no password: %v", out)
	}
	if entries, _ := os.ReadDir(os.Getenv("XDG_RUNTIME_DIR")); len(entries) != 0 {
		t.Fatalf("askpass folders left behind: %v", entries)
	}
}
