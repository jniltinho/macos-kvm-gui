package core

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// RunInfo describes a QEMU process that uses a VM's disk, whoever started it.
type RunInfo struct {
	PID       int
	Started   time.Time
	ByApp     bool   // started by macos-kvm-gui (its QMP socket is in the command line)
	VNCSocket string // the in-app display socket, when there is one
}

// Uptime is how long the VM has been running.
func (r RunInfo) Uptime() time.Duration { return time.Since(r.Started).Round(time.Second) }

// FindRunning finds the QEMU process using c's disk, including one started outside the app (for
// example by OSX-KVM's OpenCore-Boot.sh, which uses a path relative to its folder).
func FindRunning(c Config) (RunInfo, bool) {
	return runningIn("/proc", c)
}

func runningIn(proc string, c Config) (RunInfo, bool) {
	dir, err := c.WorkspaceDir()
	if err != nil {
		return RunInfo{}, false
	}
	disk := filepath.Join(dir, DiskFile)
	entries, _ := os.ReadDir(proc)
	for _, e := range entries {
		pid, err := strconv.Atoi(e.Name())
		if err != nil {
			continue
		}
		base := filepath.Join(proc, e.Name())
		if comm, _ := os.ReadFile(filepath.Join(base, "comm")); !strings.HasPrefix(string(comm), "qemu-system") {
			continue
		}
		raw, err := os.ReadFile(filepath.Join(base, "cmdline"))
		if err != nil {
			continue
		}
		args := strings.Split(strings.TrimRight(string(raw), "\x00"), "\x00")
		cwd, _ := os.Readlink(filepath.Join(base, "cwd"))
		if !usesDisk(args, disk, cwd) {
			continue
		}
		info := RunInfo{PID: pid}
		if st, err := os.Stat(base); err == nil {
			info.Started = st.ModTime() // /proc/<pid> is created when the process starts
		}
		for i, a := range args {
			if a == "-qmp" && i+1 < len(args) && strings.Contains(args[i+1], "macos-kvm-gui-") {
				info.ByApp = true
			}
			if a == "-vnc" && i+1 < len(args) {
				sock, _, _ := strings.Cut(strings.TrimPrefix(args[i+1], "unix:"), ",lock-key-sync")
				info.VNCSocket = strings.ReplaceAll(sock, ",,", ",")
			}
		}
		return info, true
	}
	return RunInfo{}, false
}

// usesDisk reports whether a QEMU command line opens disk, written absolute (with QEMU's
// ",," comma escaping) or relative to the process's working directory.
func usesDisk(args []string, disk, cwd string) bool {
	for _, a := range args {
		for _, opt := range strings.Split(strings.ReplaceAll(a, ",,", "\x01"), ",") {
			f, ok := strings.CutPrefix(opt, "file=")
			if !ok {
				continue
			}
			f = strings.Trim(strings.ReplaceAll(f, "\x01", ","), `"`)
			if !filepath.IsAbs(f) && cwd != "" {
				f = filepath.Join(cwd, f)
			}
			if filepath.Clean(f) == disk {
				return true
			}
		}
	}
	return false
}

// Preference and SetPreference store app-wide preferences (such as the theme) in the database.
func Preference(key string) string {
	db, err := openDB()
	if err != nil {
		return ""
	}
	defer db.Close()
	var v string
	db.QueryRow(`SELECT value FROM settings WHERE key = ?`, key).Scan(&v)
	return v
}

func SetPreference(key, value string) error {
	db, err := openDB()
	if err != nil {
		return err
	}
	defer db.Close()
	_, err = db.Exec(`INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value`, key, value)
	return err
}
