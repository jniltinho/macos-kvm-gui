package core

import (
	"bufio"
	"encoding/json"
	"errors"
	"net"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"
)

func argAfter(args []string, flag, prefix string) string {
	for i := 0; i+1 < len(args); i++ {
		if args[i] == flag && strings.HasPrefix(args[i+1], prefix) {
			return args[i+1]
		}
	}
	return ""
}

func TestBuildArgs(t *testing.T) {
	w := NewWorkspace("/vm/a,b")
	c := Defaults()
	cases := []struct {
		name      string
		version   string
		withMedia bool
		cpu, nic  string
	}{
		{"sonoma com mídia", "sonoma", true, "Skylake-Client", "virtio-net-pci"},
		{"sonoma sem mídia", "sonoma", false, "Skylake-Client", "virtio-net-pci"},
		{"high sierra", "high-sierra", false, "Penryn", "vmxnet3"},
	}
	for _, tc := range cases {
		c.Version = tc.version
		args := BuildArgs(c, w, "/run/q.sock", tc.withMedia)
		if !strings.HasPrefix(argAfter(args, "-cpu", ""), tc.cpu+",") {
			t.Errorf("%s: cpu %q", tc.name, argAfter(args, "-cpu", ""))
		}
		if argAfter(args, "-device", tc.nic+",netdev=net0") == "" {
			t.Errorf("%s: NIC %s ausente", tc.name, tc.nic)
		}
		hasMedia := argAfter(args, "-drive", "id=InstallMedia") != ""
		if hasMedia != tc.withMedia {
			t.Errorf("%s: mídia=%v", tc.name, hasMedia)
		}
		if got := argAfter(args, "-netdev", "user"); got != "user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22" {
			t.Errorf("%s: SSH deve ficar só em 127.0.0.1: %q", tc.name, got)
		}
		if got := argAfter(args, "-smp", ""); got != "8,sockets=1,cores=4,threads=2" {
			t.Errorf("%s: smp %q", tc.name, got)
		}
		if got := argAfter(args, "-drive", "id=MacHDD"); !strings.HasSuffix(got, "file=/vm/a,,b/mac_hdd_ng.img") {
			t.Errorf("%s: vírgula do caminho deve virar ,, : %q", tc.name, got)
		}
		if argAfter(args, "-drive", "if=pflash,format=raw,file=/vm/a,,b/"+VarsFile) == "" {
			t.Errorf("%s: deve usar a NVRAM própria", tc.name)
		}
		if !slices.Contains(args, "id=OpenCoreBoot,if=none,snapshot=on,format=qcow2,file=/vm/a,,b/OpenCore/OpenCore.qcow2") {
			t.Errorf("%s: OpenCore deve ser snapshot", tc.name)
		}
	}
}

func TestEnsureVarsNeverOverwrites(t *testing.T) {
	w := readyWorkspace(t)
	write(t, w.Path(VarsSource), "original")
	if err := ensureVars(w); err != nil {
		t.Fatal(err)
	}
	write(t, w.Path(VarsFile), "alterado-pelo-convidado")
	if err := ensureVars(w); err != nil {
		t.Fatal(err)
	}
	if b, _ := os.ReadFile(w.Path(VarsFile)); string(b) != "alterado-pelo-convidado" {
		t.Fatalf("NVRAM sobrescrita: %q", b)
	}
}

func TestExplainExit(t *testing.T) {
	busy := []string{"ioctl(KVM_CREATE_VM) failed: 16 Device or resource busy", "qemu-system-x86_64: failed to initialize kvm: Device or resource busy"}
	if m := explainExit(errors.New("exit status 1"), busy); !strings.Contains(m, "outro hypervisor") {
		t.Errorf("KVM busy: %q", m)
	}
	locked := []string{`qemu-system-x86_64: -device ide-hd,bus=sata.4,drive=MacHDD: Failed to get "write" lock`}
	if m := explainExit(errors.New("exit status 1"), locked); !strings.Contains(m, "outro QEMU") {
		t.Errorf("disco em uso: %q", m)
	}
	if m := explainExit(nil, []string{"tudo certo"}); m != "" {
		t.Errorf("saída normal não tem mensagem: %q", m)
	}
	if m := explainExit(errors.New("exit status 1"), []string{"boom"}); !strings.Contains(m, "boom") {
		t.Errorf("erro genérico deve trazer a última linha: %q", m)
	}
}

// fakeQMP é um servidor QMP mínimo: greeting, capabilities e respostas roteirizadas.
func fakeQMP(t *testing.T, handle func(cmd string, enc *json.Encoder)) string {
	sock := filepath.Join(t.TempDir(), "q.sock")
	l, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { l.Close() })
	go func() {
		c, err := l.Accept()
		if err != nil {
			return
		}
		defer c.Close()
		enc := json.NewEncoder(c)
		enc.Encode(map[string]any{"QMP": map[string]any{"version": map[string]any{}}})
		sc := bufio.NewScanner(c)
		for sc.Scan() {
			var m map[string]string
			json.Unmarshal(sc.Bytes(), &m)
			if m["execute"] == "qmp_capabilities" {
				enc.Encode(map[string]any{"return": map[string]any{}})
				continue
			}
			handle(m["execute"], enc)
			if m["execute"] == "hangup" {
				return
			}
		}
	}()
	return sock
}

func TestQMP(t *testing.T) {
	events := make(chan QMPEvent, 4)
	disconnected := make(chan struct{})
	sock := fakeQMP(t, func(cmd string, enc *json.Encoder) {
		switch cmd {
		case "system_powerdown":
			// evento chega antes da resposta: o cliente precisa separar os dois
			enc.Encode(map[string]any{"event": "POWERDOWN"})
			enc.Encode(map[string]any{"return": map[string]any{}})
		case "quebrado":
			enc.Encode(map[string]any{"error": map[string]any{"class": "GenericError", "desc": "nope"}})
		case "mudo":
			// não responde: deve dar timeout
		}
	})
	q, err := DialQMP(sock, func(e QMPEvent) { events <- e }, func() { close(disconnected) })
	if err != nil {
		t.Fatal(err)
	}
	if err := q.Execute("system_powerdown"); err != nil {
		t.Fatalf("powerdown: %v", err)
	}
	if e := <-events; e.Name != "POWERDOWN" {
		t.Fatalf("evento: %v", e)
	}
	if err := q.Execute("quebrado"); err == nil || !strings.Contains(err.Error(), "nope") {
		t.Fatalf("erro do QEMU deve ser repassado: %v", err)
	}
	start := time.Now()
	if err := q.Execute("mudo"); err == nil || time.Since(start) < qmpTimeout {
		t.Fatalf("esperava timeout: %v", err)
	}
	q.Execute("hangup")
	select {
	case <-disconnected:
	case <-time.After(2 * time.Second):
		t.Fatal("desconexão não foi notificada")
	}
	if err := q.Execute("system_powerdown"); !errors.Is(err, ErrQMPClosed) {
		t.Fatalf("após desconectar: %v", err)
	}
}

func TestVMEvents(t *testing.T) {
	var states []VMState
	var msgs []string
	v := &VM{OnState: func(s VMState, m string) { states = append(states, s); msgs = append(msgs, m) }}
	v.onEvent(QMPEvent{"BLOCK_IO_ERROR", map[string]any{"action": "report", "device": "MacHDD"}})
	if len(states) != 0 {
		t.Fatal("BLOCK_IO_ERROR sem action=stop não pausa")
	}
	v.onEvent(QMPEvent{"BLOCK_IO_ERROR", map[string]any{"action": "stop", "device": "MacHDD", "operation": "write", "nospace": true}})
	v.onEvent(QMPEvent{"STOP", nil})
	v.onEvent(QMPEvent{"RESUME", nil})
	if !slices.Equal(states, []VMState{Paused, Paused, Running}) {
		t.Fatalf("estados: %v", states)
	}
	if !strings.Contains(msgs[1], "sem espaço") {
		t.Fatalf("STOP após erro de disco deve manter a causa: %q", msgs[1])
	}
}

func TestStartRefusesWithoutDisk(t *testing.T) {
	w := readyWorkspace(t)
	c := Defaults()
	c.Workspace = w.Dir
	if err := (&VM{}).Start(c); err == nil || !strings.Contains(err.Error(), DiskFile) {
		t.Fatalf("sem disco não pode iniciar: %v", err)
	}
}
