package mcp

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/jniltinho/macos-kvm-gui/internal/core"
)

// vm_ssh runs one command in a running VM's macOS with the system's ssh, over the VM's
// 127.0.0.1 forward. A password goes to ssh through SSH_ASKPASS: ssh runs this binary (through
// a symlink named ssh-askpass, see AskpassName), which reads it from a one-shot Unix socket that
// answers only a child of that ssh. The password is never in argv, the environment or a file.

const (
	// AskpassName is the name ssh runs this binary under to ask for the password.
	AskpassName = "ssh-askpass"
	// askpassEnv names the socket the askpass helper reads the password from.
	askpassEnv = "MACOS_KVM_GUI_ASKPASS_SOCKET"
	outputCap  = 1 << 20 // per stream
)

// findRunning is core.FindRunning; tests replace it.
var findRunning = core.FindRunning

type sshIn struct {
	VM           string `json:"vm,omitempty" jsonschema:"VM name or id; default: the VM selected in the app"`
	User         string `json:"user" jsonschema:"the macOS account to log in as"`
	Command      string `json:"command" jsonschema:"the command line to run in the guest's shell"`
	Password     string `json:"password,omitempty" jsonschema:"the account's password, when the user's SSH keys or agent do not log in; never logged or stored"`
	TimeoutSec   int    `json:"timeout_sec,omitempty" jsonschema:"stop the command after this many seconds (default 60, at most 600)"`
	ResetHostKey bool   `json:"reset_host_key,omitempty" jsonschema:"forget the guest's recorded host key first (after reinstalling macOS on this port)"`
	Confirm      bool   `json:"confirm,omitempty" jsonschema:"must be true: the user agreed that commands run in the guest"`
}

type sshOut struct {
	Stdout   string `json:"stdout"`
	Stderr   string `json:"stderr"`
	ExitCode int    `json:"exit_code"`
	TimedOut bool   `json:"timed_out,omitempty"`
}

// knownHosts is the app's own known_hosts, so a guest reinstalled on the same port does not
// break the user's ~/.ssh/known_hosts.
func knownHosts() (string, error) {
	db, err := core.DBPath()
	return filepath.Join(filepath.Dir(db), "known_hosts"), err
}

// sshArgs is ssh's command line; the password is never part of it.
func sshArgs(port int, user, knownHostsFile string, password bool, command string) []string {
	batch := "yes"
	if password {
		batch = "no"
	}
	args := []string{"-p", strconv.Itoa(port), "-o", "ConnectTimeout=10", "-o", "BatchMode=" + batch,
		"-o", "StrictHostKeyChecking=accept-new", "-o", "UserKnownHostsFile=" + knownHostsFile, "-o", "LogLevel=ERROR"}
	if password {
		args = append(args, "-o", "NumberOfPasswordPrompts=1")
	}
	return append(args, user+"@127.0.0.1", "--", command)
}

func vmSSH(ctx context.Context, _ *mcp.CallToolRequest, in sshIn) (*mcp.CallToolResult, sshOut, error) {
	if !in.Confirm {
		return nil, sshOut{}, errors.New("vm_ssh runs commands in the guest as that user (they can change or delete data in macOS): ask the user, then call it with confirm: true")
	}
	if strings.TrimSpace(in.User) == "" || strings.HasPrefix(in.User, "-") || strings.ContainsAny(in.User, "@ \t\n") {
		return nil, sshOut{}, errors.New("user: give the macOS account name")
	}
	if strings.TrimSpace(in.Command) == "" {
		return nil, sshOut{}, errors.New("command is empty")
	}
	if in.TimeoutSec == 0 {
		in.TimeoutSec = 60
	}
	if in.TimeoutSec < 1 || in.TimeoutSec > 600 {
		return nil, sshOut{}, errors.New("timeout_sec: use 1 to 600 seconds")
	}
	e, err := findVM(in.VM)
	if err != nil {
		return nil, sshOut{}, err
	}
	r, ok := findRunning(e.Config)
	if !ok {
		return nil, sshOut{}, fmt.Errorf("%s is not running (vm_start starts it)", e.DisplayName())
	}
	if r.SSHPort == 0 { // the port of this run, not the stored one
		return nil, sshOut{}, fmt.Errorf("%s has no SSH forward on 127.0.0.1 (no NAT adapter): use the VM's address on its network", e.DisplayName())
	}
	kh, err := knownHosts()
	if err != nil {
		return nil, sshOut{}, err
	}
	if err := os.MkdirAll(filepath.Dir(kh), 0o755); err != nil {
		return nil, sshOut{}, err
	}
	if in.ResetHostKey && fileExists(kh) {
		if out, err := exec.Command("ssh-keygen", "-R", fmt.Sprintf("[127.0.0.1]:%d", r.SSHPort), "-f", kh).CombinedOutput(); err != nil {
			return nil, sshOut{}, fmt.Errorf("forgetting the host key: %v: %s", err, out)
		}
		os.Remove(kh + ".old")
	}
	out, err := runSSH(ctx, sshArgs(r.SSHPort, in.User, kh, in.Password != "", in.Command), in.Password, time.Duration(in.TimeoutSec)*time.Second)
	if err != nil {
		return nil, sshOut{}, err
	}
	return nil, out, nil
}

func fileExists(p string) bool { _, err := os.Stat(p); return err == nil }

// capped keeps the first outputCap bytes written to it.
type capped struct {
	buf       bytes.Buffer
	truncated bool
}

func (c *capped) Write(p []byte) (int, error) {
	if room := outputCap - c.buf.Len(); len(p) > room {
		c.buf.Write(p[:max(room, 0)])
		c.truncated = true
	} else {
		c.buf.Write(p)
	}
	return len(p), nil
}

func (c *capped) String() string {
	if c.truncated {
		return c.buf.String() + "\n[truncated at 1 MiB]"
	}
	return c.buf.String()
}

// runSSH runs ssh with args in its own session (no terminal to ask on, and one process group to
// kill on timeout), answering a password prompt through the askpass socket.
func runSSH(ctx context.Context, args []string, password string, timeout time.Duration) (sshOut, error) {
	ctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	var stdout, stderr capped
	cmd := exec.CommandContext(ctx, "ssh", args...)
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	cmd.Cancel = func() error { return syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL) }
	cmd.WaitDelay = 2 * time.Second
	var env []string
	for _, kv := range os.Environ() {
		if !strings.HasPrefix(kv, "SSH_ASKPASS") && !strings.HasPrefix(kv, askpassEnv+"=") {
			env = append(env, kv)
		}
	}
	var ap *askpassServer
	if password != "" {
		var err error
		if ap, err = newAskpass(password); err != nil {
			return sshOut{}, err
		}
		defer ap.close()
		env = append(env, "SSH_ASKPASS="+ap.helper, "SSH_ASKPASS_REQUIRE=force", askpassEnv+"="+ap.sock)
	}
	cmd.Env = env
	if err := cmd.Start(); err != nil {
		return sshOut{}, fmt.Errorf("running ssh: %w", err)
	}
	if ap != nil {
		go ap.serve(cmd.Process.Pid)
	}
	err := cmd.Wait()
	out := sshOut{Stdout: stdout.String(), Stderr: stderr.String(), ExitCode: cmd.ProcessState.ExitCode()}
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		out.TimedOut = true
		out.Stderr += fmt.Sprintf("\n[stopped after %s]", timeout)
		return out, nil
	}
	var exitErr *exec.ExitError
	if err != nil && !errors.As(err, &exitErr) {
		return out, err
	}
	return out, nil
}

// askpassServer hands the password once, over a Unix socket in a fresh 0700 directory, to the
// askpass helper that ssh starts.
type askpassServer struct {
	dir, sock, helper string
	l                 *net.UnixListener
	password          string
}

func newAskpass(password string) (*askpassServer, error) {
	base := os.Getenv("XDG_RUNTIME_DIR")
	if base == "" {
		base = os.TempDir()
	}
	dir, err := os.MkdirTemp(base, "macos-kvm-gui-askpass-") // 0700
	if err != nil {
		return nil, err
	}
	a := &askpassServer{dir: dir, sock: filepath.Join(dir, "s"), helper: filepath.Join(dir, AskpassName), password: password}
	self, err := os.Executable()
	if err == nil {
		err = os.Symlink(self, a.helper)
	}
	if err == nil {
		a.l, err = net.ListenUnix("unix", &net.UnixAddr{Name: a.sock, Net: "unix"})
	}
	if err != nil {
		os.RemoveAll(dir)
		return nil, err
	}
	return a, nil
}

// serve answers the first connection from a process whose parent is sshPID (checked with
// SO_PEERCRED), then stops; other connections get nothing.
func (a *askpassServer) serve(sshPID int) {
	for {
		c, err := a.l.AcceptUnix()
		if err != nil {
			return
		}
		if peerParent(c) == sshPID {
			io.WriteString(c, a.password)
			c.Close()
			a.close()
			return
		}
		c.Close()
	}
}

func (a *askpassServer) close() {
	a.l.Close()
	os.RemoveAll(a.dir)
}

// peerParent is the parent pid of the process on the other end of c, or -1.
func peerParent(c *net.UnixConn) int {
	raw, err := c.SyscallConn()
	if err != nil {
		return -1
	}
	var cred *syscall.Ucred
	raw.Control(func(fd uintptr) {
		cred, err = syscall.GetsockoptUcred(int(fd), syscall.SOL_SOCKET, syscall.SO_PEERCRED)
	})
	if err != nil || cred == nil || int(cred.Uid) != os.Getuid() {
		return -1
	}
	return parentOf(int(cred.Pid))
}

// parentOf reads a process's parent pid from /proc/<pid>/stat.
func parentOf(pid int) int {
	b, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		return -1
	}
	// pid (comm) state ppid ...: comm may contain spaces and parentheses.
	i := bytes.LastIndexByte(b, ')')
	if i < 0 {
		return -1
	}
	f := strings.Fields(string(b[i+1:]))
	if len(f) < 2 {
		return -1
	}
	p, err := strconv.Atoi(f[1])
	if err != nil {
		return -1
	}
	return p
}

// Askpass is the ssh-askpass helper mode: it prints the password the MCP server's socket gives.
func Askpass(w io.Writer) error {
	sock := os.Getenv(askpassEnv)
	if sock == "" {
		return errors.New("ssh-askpass is run by the MCP server's vm_ssh only")
	}
	c, err := net.DialTimeout("unix", sock, 5*time.Second)
	if err != nil {
		return err
	}
	defer c.Close()
	c.SetDeadline(time.Now().Add(10 * time.Second))
	b, err := io.ReadAll(io.LimitReader(c, 64<<10))
	if err != nil {
		return err
	}
	if len(b) == 0 {
		return errors.New("no password was given")
	}
	_, err = fmt.Fprintln(w, string(b))
	return err
}
