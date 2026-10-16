package core

import (
	"archive/tar"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// A VM bundle is an uncompressed tar with everything a VM needs to run elsewhere: its settings
// (a manifest), the macOS disk, the UEFI NVRAM, the SMBIOS identity and, when present, the
// install media. OpenCore and OVMF are not included: the app regenerates them at every start.

const bundleManifest = "macos-kvm-gui-vm.json"

// bundleFiles are the only workspace files a bundle may contain, in order. Import refuses any
// other name, so a crafted bundle cannot write outside the new workspace.
var bundleFiles = []string{DiskFile, VarsFile, identityFile, MediaFile, mediaMarker}

type manifest struct {
	Format int    `json:"format"`
	App    string `json:"app"`
	Config Config `json:"config"`
}

// ExportBundle writes e as a bundle to dst (through a temporary file). The VM must be stopped:
// the workspace lock is held while the disk is copied.
func ExportBundle(ctx context.Context, e VMEntry, dst string, progress func(done, total int64)) error {
	dir, err := e.Config.WorkspaceDir()
	if err != nil {
		return err
	}
	if _, ok := FindRunning(e.Config); ok {
		return fmt.Errorf("shut down %s before exporting it", e.DisplayName())
	}
	w := NewWorkspace(dir)
	if !w.HasDisk() {
		return fmt.Errorf("%s has no disk yet: nothing to export", e.DisplayName())
	}
	return w.withLock(func() error {
		var files []string
		var total int64
		for _, f := range bundleFiles {
			if st, err := os.Stat(w.Path(f)); err == nil && st.Mode().IsRegular() {
				files = append(files, f)
				total += st.Size()
			}
		}
		tmp := dst + ".part"
		out, err := os.Create(tmp)
		if err != nil {
			return err
		}
		ok := false
		defer func() {
			if !ok {
				out.Close()
				os.Remove(tmp)
			}
		}()
		tw := tar.NewWriter(out)
		m, _ := json.MarshalIndent(manifest{Format: 1, App: "macos-kvm-gui", Config: e.Config}, "", "  ")
		if err := tw.WriteHeader(&tar.Header{Name: bundleManifest, Mode: 0o644, Size: int64(len(m))}); err != nil {
			return err
		}
		if _, err := tw.Write(m); err != nil {
			return err
		}
		var done int64
		for _, f := range files {
			if err := addFile(ctx, tw, w.Path(f), f, &done, total, progress); err != nil {
				return fmt.Errorf("exporting %s: %w", f, err)
			}
		}
		if err := tw.Close(); err != nil {
			return err
		}
		if err := out.Close(); err != nil {
			return err
		}
		ok = true
		return os.Rename(tmp, dst)
	})
}

func addFile(ctx context.Context, tw *tar.Writer, path, name string, done *int64, total int64, progress func(int64, int64)) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	st, err := f.Stat()
	if err != nil {
		return err
	}
	if err := tw.WriteHeader(&tar.Header{Name: name, Mode: 0o644, Size: st.Size(), ModTime: st.ModTime()}); err != nil {
		return err
	}
	_, err = io.Copy(tw, &progressReader{ctx: ctx, r: f, done: done, total: total, report: progress})
	return err
}

// progressReader reports bytes copied and stops when ctx is cancelled.
type progressReader struct {
	ctx    context.Context
	r      io.Reader
	done   *int64
	total  int64
	report func(int64, int64)
	last   int64
}

func (p *progressReader) Read(b []byte) (int, error) {
	if err := p.ctx.Err(); err != nil {
		return 0, err
	}
	n, err := p.r.Read(b)
	*p.done += int64(n)
	if p.report != nil && (*p.done-p.last > 64<<20 || err == io.EOF) {
		p.last = *p.done
		p.report(*p.done, p.total)
	}
	return n, err
}

// ImportBundle unpacks a bundle into workspace (which must not exist or be empty) and adds the
// VM. A name already in use gets " (imported)" appended. On any error nothing is left behind.
func ImportBundle(ctx context.Context, src, workspace string, progress func(done, total int64)) (e VMEntry, err error) {
	if !filepath.IsAbs(workspace) {
		return VMEntry{}, &FieldError{"workspace", "must be an absolute path"}
	}
	// The lock keeps two imports into the same folder from cleaning up each other's files.
	err = NewWorkspace(workspace).withLock(func() error {
		e, err = importBundle(ctx, src, workspace, progress)
		return err
	})
	return e, err
}

func importBundle(ctx context.Context, src, workspace string, progress func(done, total int64)) (VMEntry, error) {
	if err := requireEmpty(workspace); err != nil {
		return VMEntry{}, err
	}
	in, err := os.Open(src)
	if err != nil {
		return VMEntry{}, err
	}
	defer in.Close()
	st, _ := in.Stat()
	tr := tar.NewReader(in)

	h, err := tr.Next()
	if err != nil || h.Name != bundleManifest {
		return VMEntry{}, errors.New("not a macos-kvm-gui VM bundle (no manifest)")
	}
	var m manifest
	if err := json.NewDecoder(io.LimitReader(tr, 1<<20)).Decode(&m); err != nil || m.Format != 1 {
		return VMEntry{}, fmt.Errorf("unsupported bundle manifest: %v", err)
	}

	created := !exists(workspace)
	if err := os.MkdirAll(filepath.Join(workspace, BootDir), 0o755); err != nil {
		return VMEntry{}, err
	}
	cleanup := func() {
		for _, f := range bundleFiles {
			os.Remove(filepath.Join(workspace, f))
		}
		os.RemoveAll(filepath.Join(workspace, BootDir))
		if created {
			os.Remove(workspace)
		}
	}
	allowed := map[string]bool{}
	for _, f := range bundleFiles {
		allowed[f] = true
	}
	var done int64
	for {
		h, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			cleanup()
			return VMEntry{}, err
		}
		if !allowed[h.Name] || h.Typeflag != tar.TypeReg {
			cleanup()
			return VMEntry{}, fmt.Errorf("refusing unexpected bundle entry %q", h.Name)
		}
		out, err := os.OpenFile(filepath.Join(workspace, h.Name), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o644)
		if err != nil {
			cleanup()
			return VMEntry{}, err
		}
		_, err = io.Copy(out, &progressReader{ctx: ctx, r: io.LimitReader(tr, h.Size), done: &done, total: st.Size(), report: progress})
		if cerr := out.Close(); err == nil {
			err = cerr
		}
		if err != nil {
			cleanup()
			return VMEntry{}, err
		}
	}
	if !exists(filepath.Join(workspace, DiskFile)) {
		cleanup()
		return VMEntry{}, errors.New("the bundle has no macOS disk")
	}
	if err := checkStandaloneDisk(filepath.Join(workspace, DiskFile)); err != nil {
		cleanup()
		return VMEntry{}, err
	}
	c := m.Config
	c.Workspace, c.GPU = workspace, "" // a GPU address from another host means nothing here
	e, err := CreateVM(c)
	var fe *FieldError
	if errors.As(err, &fe) && fe.Field == "name" {
		c.Name = strings.TrimSpace(c.Name) + " (imported)"
		e, err = CreateVM(c)
	}
	if err != nil {
		cleanup()
		return VMEntry{}, err
	}
	return e, nil
}

// checkStandaloneDisk refuses a qcow2 that points at other files (a backing file or an external
// data file): a crafted bundle could otherwise make QEMU read or write host files.
func checkStandaloneDisk(path string) error {
	out, err := exec.Command("qemu-img", "info", "--output=json", "-U", path).Output()
	if err != nil {
		return fmt.Errorf("checking the imported disk: %w", err)
	}
	var info struct {
		Format  string `json:"format"`
		Backing string `json:"backing-filename"`
		Spec    struct {
			Data struct {
				DataFile string `json:"data-file"`
			} `json:"data"`
		} `json:"format-specific"`
	}
	if err := json.Unmarshal(out, &info); err != nil {
		return fmt.Errorf("checking the imported disk: %w", err)
	}
	if info.Format != "qcow2" || info.Backing != "" || info.Spec.Data.DataFile != "" {
		return errors.New("the bundle's disk is not a standalone qcow2 (it references other files): refusing it")
	}
	return nil
}

// ImportSettings adds a VM from a settings file: an export, a --config file or a workspace
// snapshot (vm.json). The workspace it names is used as is.
func ImportSettings(path string) (VMEntry, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return VMEntry{}, err
	}
	var wrapped struct {
		Config *Config `json:"config"`
	}
	c := Defaults()
	if json.Unmarshal(b, &wrapped) == nil && wrapped.Config != nil {
		c = *wrapped.Config
	} else if err := json.Unmarshal(b, &c); err != nil {
		return VMEntry{}, fmt.Errorf("%s is not a VM settings file: %w", path, err)
	}
	if strings.TrimSpace(c.Name) == "" {
		c.Name = strings.TrimSuffix(filepath.Base(path), filepath.Ext(path))
	}
	e, err := CreateVM(c)
	var fe *FieldError
	if errors.As(err, &fe) && fe.Field == "name" {
		c.Name += " (imported)"
		e, err = CreateVM(c)
	}
	return e, err
}
