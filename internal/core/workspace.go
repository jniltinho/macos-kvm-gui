package core

import (
	"bufio"
	"context"
	"crypto/rsa"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"syscall"
)

const (
	MediaFile   = "BaseSystem.img"
	DiskFile    = "mac_hdd_ng.img"
	mediaMarker = ".macos-kvm-gui-media"
	stagingDir  = ".macos-kvm-gui-staging"
	MinDiskGiB  = 40
	minFreeGiB  = 4
	gib         = int64(1) << 30
)

var ErrBusy = errors.New("workspace in use by another operation or another app instance")

type Workspace struct {
	Dir string
	key *rsa.PublicKey // nil = Apple's key; replaced only in tests
}

func NewWorkspace(dir string) Workspace { return Workspace{Dir: dir} }

func (w Workspace) Path(name string) string { return filepath.Join(w.Dir, name) }

// Lock takes an exclusive flock per workspace. The file lives outside the workspace so
// the workspace can still be created by the locked operation; the kernel releases it if the app dies.
func (w Workspace) Lock() (unlock func(), err error) {
	base := os.Getenv("XDG_RUNTIME_DIR")
	if base == "" {
		base = os.TempDir()
	}
	sum := sha256.Sum256([]byte(realPath(w.Dir)))
	f, err := os.OpenFile(filepath.Join(base, "macos-kvm-gui-"+hex.EncodeToString(sum[:8])+".lock"), os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		f.Close()
		if errors.Is(err, syscall.EWOULDBLOCK) {
			return nil, ErrBusy
		}
		return nil, err
	}
	return func() { f.Close() }, nil
}

func (w Workspace) withLock(fn func() error) error {
	unlock, err := w.Lock()
	if err != nil {
		return err
	}
	defer unlock()
	return fn()
}

// Exists reports whether the workspace directory exists.
func (w Workspace) Exists() bool {
	st, err := os.Stat(w.Dir)
	return err == nil && st.IsDir()
}

// Prepare creates the workspace and extracts the embedded boot files into it.
func (w Workspace) Prepare(c Config) error {
	return w.withLock(func() error {
		if err := os.MkdirAll(w.Dir, 0o755); err != nil {
			return err
		}
		return w.PrepareBoot(c, w.HasMedia())
	})
}

// runStream runs a command, sending each output chunk (split on \r or \n) to log.
func runStream(ctx context.Context, dir string, log func(string), name string, args ...string) error {
	cmd := exec.CommandContext(ctx, name, args...)
	cmd.Dir = dir
	pr, pw := io.Pipe()
	cmd.Stdout, cmd.Stderr = pw, pw
	if err := cmd.Start(); err != nil {
		return err
	}
	done := make(chan struct{})
	var last string
	go func() {
		defer close(done)
		sc := bufio.NewScanner(pr)
		sc.Split(scanCRLF)
		for sc.Scan() {
			if s := strings.TrimSpace(sc.Text()); s != "" {
				last = s
				if log != nil {
					log(s)
				}
			}
		}
		io.Copy(io.Discard, pr)
	}()
	err := cmd.Wait()
	pw.Close()
	<-done
	if err != nil && last != "" {
		return fmt.Errorf("%s: %w (%s)", name, err, last)
	}
	return err
}

// scanCRLF splits on \n or \r, because progress bars rewrite the line with \r.
func scanCRLF(data []byte, atEOF bool) (int, []byte, error) {
	for i, b := range data {
		if b == '\n' || b == '\r' {
			return i + 1, data[:i], nil
		}
	}
	if atEOF && len(data) > 0 {
		return len(data), data, nil
	}
	return 0, nil, nil
}

// Media preparation stages.
const (
	StageChunklist = "Downloading chunklist"
	StageImage     = "Downloading image"
	StageVerify    = "Verifying signature and hashes"
	StageConvert   = "Converting to raw image"
)

// Progress: Pct < 0 means indeterminate.
type Progress struct {
	Stage string
	Pct   float64
}

var convertPctRe = regexp.MustCompile(`\((\d+(?:\.\d+)?)/100%\)`)

// parseConvertProgress reads the percentage printed by qemu-img convert -p.
func parseConvertProgress(line string) (float64, bool) {
	m := convertPctRe.FindStringSubmatch(line)
	if m == nil {
		return 0, false
	}
	pct, err := strconv.ParseFloat(m[1], 64)
	return pct, err == nil
}

func freeBytes(dir string) (int64, error) {
	var st syscall.Statfs_t
	if err := syscall.Statfs(dir, &st); err != nil {
		return 0, err
	}
	return int64(st.Bavail) * st.Bsize, nil
}

// DownloadMedia downloads, verifies and publishes the recovery for the requested version.
// On any failure the previous BaseSystem.img stays intact.
func (w Workspace) DownloadMedia(ctx context.Context, version string, progress func(Progress), log func(string)) error {
	return w.withLock(func() error {
		if err := os.MkdirAll(w.Dir, 0o755); err != nil {
			return err
		}
		if free, err := freeBytes(w.Dir); err == nil && free < minFreeGiB*gib {
			return fmt.Errorf("not enough free space: %d GiB needed and %.1f GiB available", minFreeGiB, float64(free)/float64(gib))
		}
		staging := w.Path(stagingDir)
		os.RemoveAll(staging)
		if err := os.Mkdir(staging, 0o755); err != nil {
			return err
		}
		defer os.RemoveAll(staging)

		progress(Progress{StageChunklist, -1})
		stages := map[string]string{"chunklist": StageChunklist, "image": StageImage}
		dmg, cnk, err := DownloadRecovery(ctx, version, staging, func(stage string, pct float64) {
			progress(Progress{stages[stage], pct})
		})
		if err != nil {
			return fmt.Errorf("download failed: %w", err)
		}

		err = verifyImage(dmg, cnk, w.verifyKey(), func(done, total int) {
			progress(Progress{StageVerify, float64(done) * 100 / float64(total)})
		})
		if err != nil {
			return fmt.Errorf("verification failed: %w", err)
		}

		progress(Progress{StageConvert, 0})
		report := func(line string) {
			if pct, ok := parseConvertProgress(line); ok {
				progress(Progress{StageConvert, pct}) // percentages go only to the bar
			} else if log != nil {
				log(line)
			}
		}
		tmpImg := filepath.Join(staging, MediaFile)
		if err := runStream(ctx, "", report, "qemu-img", "convert", "-p", "-f", "dmg", "-O", "raw", dmg, tmpImg); err != nil {
			return fmt.Errorf("conversion failed (if qemu-img does not support this compression, install dmg2img): %w", err)
		}
		// The old marker goes before the rename: a stale version never labels the new media.
		os.Remove(w.Path(mediaMarker))
		if err := os.Rename(tmpImg, w.Path(MediaFile)); err != nil {
			return err
		}
		// After the rename the new media is already valid; failing here only leaves the version "unknown".
		if err := os.WriteFile(w.Path(mediaMarker), []byte(version+"\n"), 0o644); err != nil && log != nil {
			log("warning: could not record the media version: " + err.Error())
		}
		return nil
	})
}

func (w Workspace) verifyKey() *rsa.PublicKey {
	if w.key != nil {
		return w.key
	}
	return appleEFIKey
}

// MediaVersion is the recorded version of BaseSystem.img ("" if there is no media).
func (w Workspace) MediaVersion() string {
	if !w.HasMedia() {
		return ""
	}
	v, err := os.ReadFile(w.Path(mediaMarker))
	if err != nil {
		return "unknown"
	}
	return strings.TrimSpace(string(v))
}

func (w Workspace) HasMedia() bool { return exists(w.Path(MediaFile)) }
func (w Workspace) HasDisk() bool  { return exists(w.Path(DiskFile)) }

// realPath resolves symlinks so two spellings of one folder get one lock; a missing path is
// resolved as far as it exists. Symlinks are resolved before "..", as the kernel does, so the
// path is not cleaned first.
func realPath(p string) string {
	if r, err := filepath.EvalSymlinks(p); err == nil {
		return r
	}
	if dir, base := filepath.Split(p); dir != "" && dir != p {
		return filepath.Join(realPath(dir), base)
	}
	return filepath.Clean(p)
}

func exists(p string) bool {
	_, err := os.Stat(p)
	return err == nil
}

// Allocated returns the bytes actually used on the host (sparse files count less).
func Allocated(path string) int64 {
	var st syscall.Stat_t
	if syscall.Stat(path, &st) != nil {
		return 0
	}
	return st.Blocks * 512
}

// CreateDisk creates the thin qcow2. A non-empty warning = disk created, with a caveat.
func (w Workspace) CreateDisk(ctx context.Context, sizeGiB int) (warning string, err error) {
	if sizeGiB < MinDiskGiB {
		return "", fmt.Errorf("the disk must be at least %d GiB", MinDiskGiB)
	}
	err = w.withLock(func() error {
		if w.HasDisk() {
			return fmt.Errorf("%s already exists; nothing was changed", DiskFile)
		}
		if err := runStream(ctx, "", nil, "qemu-img", "create", "-f", "qcow2", w.Path(DiskFile), strconv.Itoa(sizeGiB)+"G"); err != nil {
			return err
		}
		if free, err := freeBytes(w.Dir); err == nil && free < int64(sizeGiB)*gib {
			warning = fmt.Sprintf("the host has only %.1f GiB free: the disk is thin and the VM may stop when space runs out", float64(free)/float64(gib))
		}
		return nil
	})
	return warning, err
}

// RemoveMedia deletes BaseSystem.img and returns the bytes freed.
func (w Workspace) RemoveMedia() (freed int64, err error) {
	err = w.withLock(func() error {
		p := w.Path(MediaFile)
		if !exists(p) {
			return errors.New("there is no install media to remove")
		}
		freed = Allocated(p)
		if err := os.Remove(p); err != nil {
			return err
		}
		os.Remove(w.Path(mediaMarker))
		return nil
	})
	return freed, err
}
