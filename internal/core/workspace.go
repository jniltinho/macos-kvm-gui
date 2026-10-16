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
	RepoURL     = "https://github.com/kholia/OSX-KVM.git"
	MediaFile   = "BaseSystem.img"
	DiskFile    = "mac_hdd_ng.img"
	VarsFile    = "macos-kvm-gui-vars.fd"
	VarsSource  = "OVMF_VARS-1920x1080.fd"
	OVMFCode    = "OVMF_CODE_4M.fd"
	OpenCore    = "OpenCore/OpenCore.qcow2"
	FetchScript = "fetch-macOS-v2.py"
	mediaMarker = ".macos-kvm-gui-media"
	stagingDir  = ".macos-kvm-gui-staging"
	MinDiskGiB  = 40
	minFreeGiB  = 4
	gib         = int64(1) << 30
)

// RequiredFiles são os arquivos do OSX-KVM que o app usa.
var RequiredFiles = []string{OpenCore, OVMFCode, VarsSource, FetchScript}

var ErrBusy = errors.New("workspace em uso por outra operação ou outra instância do app")

type Workspace struct {
	Dir string
	key *rsa.PublicKey // nil = chave da Apple; trocada só em testes
}

func NewWorkspace(dir string) Workspace { return Workspace{Dir: dir} }

func (w Workspace) Path(name string) string { return filepath.Join(w.Dir, name) }

// Lock pega um flock exclusivo por workspace. O arquivo fica fora do workspace
// para não impedir o git clone em diretório vazio; o kernel solta se o app morrer.
func (w Workspace) Lock() (unlock func(), err error) {
	base := os.Getenv("XDG_RUNTIME_DIR")
	if base == "" {
		base = os.TempDir()
	}
	sum := sha256.Sum256([]byte(filepath.Clean(w.Dir)))
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

type WorkspaceState int

const (
	WorkspaceEmpty WorkspaceState = iota // inexistente ou vazio: pode clonar
	WorkspaceReady
	WorkspaceIncomplete // não vazio e faltando arquivos: não mexemos
)

// Classify diz o estado do workspace e, se incompleto, quais arquivos faltam.
func (w Workspace) Classify() (WorkspaceState, []string) {
	entries, err := os.ReadDir(w.Dir)
	if errors.Is(err, os.ErrNotExist) || err == nil && len(entries) == 0 {
		return WorkspaceEmpty, nil
	}
	var missing []string
	for _, f := range RequiredFiles {
		if _, err := os.Stat(w.Path(f)); err != nil {
			missing = append(missing, f)
		}
	}
	if len(missing) > 0 {
		return WorkspaceIncomplete, missing
	}
	return WorkspaceReady, nil
}

// Prepare clona o OSX-KVM (raso) se o workspace estiver vazio.
func (w Workspace) Prepare(ctx context.Context, log func(string)) error {
	return w.withLock(func() error {
		switch st, missing := w.Classify(); st {
		case WorkspaceReady:
			return nil
		case WorkspaceIncomplete:
			return fmt.Errorf("%s não está vazio e faltam: %s", w.Dir, strings.Join(missing, ", "))
		}
		if err := os.MkdirAll(filepath.Dir(w.Dir), 0o755); err != nil {
			return err
		}
		return runStream(ctx, "", log, "git", "clone", "--depth", "1", "--recursive", "--shallow-submodules", "--progress", RepoURL, w.Dir)
	})
}

// runStream roda um comando enviando cada trecho de saída (separado por \r ou \n) para log.
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

// scanCRLF divide em \n ou \r, porque barras de progresso reescrevem a linha com \r.
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

// Etapas do preparo da mídia.
const (
	StageChunklist = "Baixando chunklist"
	StageImage     = "Baixando imagem"
	StageVerify    = "Verificando assinatura e hashes"
	StageConvert   = "Convertendo para imagem raw"
)

// Progress: Pct < 0 significa indeterminado.
type Progress struct {
	Stage string
	Pct   float64
}

var (
	downloadPctRe = regexp.MustCompile(`([\d.]+)% downloaded`)
	convertPctRe  = regexp.MustCompile(`\((\d+(?:\.\d+)?)/100%\)`)
)

// parseProgress traduz um trecho de saída do fetch-macOS-v2.py ou do qemu-img -p.
// Devolve ok=false quando o trecho não muda o progresso.
func parseProgress(stage, line string) (Progress, bool) {
	switch {
	case strings.HasPrefix(line, "Saving ") && strings.Contains(line, ".chunklist"):
		return Progress{StageChunklist, -1}, true
	case strings.HasPrefix(line, "Saving ") && strings.Contains(line, ".dmg"):
		return Progress{StageImage, -1}, true
	}
	re := downloadPctRe
	if stage == StageConvert {
		re = convertPctRe
	}
	if m := re.FindStringSubmatch(line); m != nil {
		if pct, err := strconv.ParseFloat(m[1], 64); err == nil {
			return Progress{stage, pct}, true
		}
	}
	return Progress{}, false
}

func freeBytes(dir string) (int64, error) {
	var st syscall.Statfs_t
	if err := syscall.Statfs(dir, &st); err != nil {
		return 0, err
	}
	return int64(st.Bavail) * st.Bsize, nil
}

// DownloadMedia baixa, verifica e publica a recovery da versão pedida.
// Em qualquer falha a BaseSystem.img anterior fica intacta.
func (w Workspace) DownloadMedia(ctx context.Context, version string, progress func(Progress), log func(string)) error {
	return w.withLock(func() error {
		if st, _ := w.Classify(); st != WorkspaceReady {
			return errors.New("prepare o workspace antes de baixar a mídia")
		}
		if free, err := freeBytes(w.Dir); err == nil && free < minFreeGiB*gib {
			return fmt.Errorf("espaço livre insuficiente: são necessários %d GiB e há %.1f GiB", minFreeGiB, float64(free)/float64(gib))
		}
		staging := w.Path(stagingDir)
		os.RemoveAll(staging)
		if err := os.Mkdir(staging, 0o755); err != nil {
			return err
		}
		defer os.RemoveAll(staging)

		stage := StageChunklist
		report := func(line string) {
			if p, ok := parseProgress(stage, line); ok {
				stage = p.Stage
				progress(p)
				if p.Pct >= 0 {
					return // percentuais vão só para a barra, não para o log
				}
			}
			if log != nil {
				log(line)
			}
		}
		progress(Progress{stage, -1})
		// O código de saída do script não prova nada (sem TTY ele falha a verificação e sai 0);
		// a verificação de verdade é a nossa, logo abaixo.
		if err := runStream(ctx, staging, report, "python3", w.Path(FetchScript), "-s", version); err != nil {
			return fmt.Errorf("download falhou: %w", err)
		}
		dmg, cnk := glob1(staging, "*.dmg"), glob1(staging, "*.chunklist")
		if dmg == "" || cnk == "" {
			return errors.New("o download não produziu o .dmg e o .chunklist")
		}

		stage = StageVerify
		err := verifyImage(dmg, cnk, w.verifyKey(), func(done, total int) {
			progress(Progress{StageVerify, float64(done) * 100 / float64(total)})
		})
		if err != nil {
			return fmt.Errorf("verificação falhou: %w", err)
		}

		stage = StageConvert
		progress(Progress{stage, 0})
		tmpImg := filepath.Join(staging, MediaFile)
		if err := runStream(ctx, "", report, "qemu-img", "convert", "-p", "-f", "dmg", "-O", "raw", dmg, tmpImg); err != nil {
			return fmt.Errorf("conversão falhou (se o qemu-img não suportar esta compressão, instale o dmg2img): %w", err)
		}
		// Marcador antigo sai antes do rename: uma versão velha nunca rotula a mídia nova.
		os.Remove(w.Path(mediaMarker))
		if err := os.Rename(tmpImg, w.Path(MediaFile)); err != nil {
			return err
		}
		// Após o rename a mídia nova já é válida; falhar aqui só deixa a versão "desconhecida".
		if err := os.WriteFile(w.Path(mediaMarker), []byte(version+"\n"), 0o644); err != nil && log != nil {
			log("aviso: não foi possível registrar a versão da mídia: " + err.Error())
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

func glob1(dir, pattern string) string {
	m, _ := filepath.Glob(filepath.Join(dir, pattern))
	if len(m) == 0 {
		return ""
	}
	return m[0]
}

// MediaVersion é a versão registrada da BaseSystem.img ("" se não houver mídia).
func (w Workspace) MediaVersion() string {
	if !w.HasMedia() {
		return ""
	}
	v, err := os.ReadFile(w.Path(mediaMarker))
	if err != nil {
		return "desconhecida"
	}
	return strings.TrimSpace(string(v))
}

func (w Workspace) HasMedia() bool { return exists(w.Path(MediaFile)) }
func (w Workspace) HasDisk() bool  { return exists(w.Path(DiskFile)) }

func exists(p string) bool {
	_, err := os.Stat(p)
	return err == nil
}

// Allocated devolve os bytes realmente ocupados no host (arquivos esparsos contam menos).
func Allocated(path string) int64 {
	var st syscall.Stat_t
	if syscall.Stat(path, &st) != nil {
		return 0
	}
	return st.Blocks * 512
}

// CreateDisk cria o qcow2 thin. warning não vazio = disco criado, mas com ressalva.
func (w Workspace) CreateDisk(ctx context.Context, sizeGiB int) (warning string, err error) {
	if sizeGiB < MinDiskGiB {
		return "", fmt.Errorf("o disco precisa ter pelo menos %d GiB", MinDiskGiB)
	}
	err = w.withLock(func() error {
		if w.HasDisk() {
			return fmt.Errorf("%s já existe; nada foi alterado", DiskFile)
		}
		if err := runStream(ctx, "", nil, "qemu-img", "create", "-f", "qcow2", w.Path(DiskFile), strconv.Itoa(sizeGiB)+"G"); err != nil {
			return err
		}
		if free, err := freeBytes(w.Dir); err == nil && free < int64(sizeGiB)*gib {
			warning = fmt.Sprintf("o host tem só %.1f GiB livres: o disco é thin e a VM pode parar por falta de espaço", float64(free)/float64(gib))
		}
		return nil
	})
	return warning, err
}

// RemoveMedia apaga a BaseSystem.img e devolve os bytes liberados.
func (w Workspace) RemoveMedia() (freed int64, err error) {
	err = w.withLock(func() error {
		p := w.Path(MediaFile)
		if !exists(p) {
			return errors.New("não há mídia de instalação para remover")
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
