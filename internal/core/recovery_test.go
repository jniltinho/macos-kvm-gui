package core

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"testing"
)

type fakeRecovery struct {
	t         *testing.T
	srv       *httptest.Server
	mu        sync.Mutex
	body      string
	cookies   map[string]string // request path -> Cookie header
	status    int               // status for RecoveryImage; 0 = 200
	shortDMG  bool              // announce more bytes than sent for the DMG
	chunklist string
	dmg       string
}

func newFakeRecovery(t *testing.T) *fakeRecovery {
	f := &fakeRecovery{t: t, cookies: map[string]string{}, chunklist: "CNKL-data", dmg: strings.Repeat("D", 3<<20)}
	f.srv = httptest.NewServer(http.HandlerFunc(f.serve))
	t.Cleanup(f.srv.Close)
	old := recoveryBaseURL
	recoveryBaseURL = f.srv.URL
	t.Cleanup(func() { recoveryBaseURL = old })
	return f
}

func (f *fakeRecovery) serve(w http.ResponseWriter, r *http.Request) {
	f.mu.Lock()
	f.cookies[r.URL.Path] = r.Header.Get("Cookie")
	f.mu.Unlock()
	if r.Header.Get("User-Agent") != "InternetRecovery/1.0" {
		f.t.Errorf("%s: User-Agent %q", r.URL.Path, r.Header.Get("User-Agent"))
	}
	switch r.URL.Path {
	case "/":
		if r.Host != "osrecovery.apple.com" {
			f.t.Errorf("session Host %q", r.Host)
		}
		w.Header().Add("Set-Cookie", "session=abc123; Path=/; HttpOnly")
	case "/InstallationPayload/RecoveryImage":
		if r.Method != http.MethodPost || r.Header.Get("Content-Type") != "text/plain" {
			f.t.Errorf("info: method %s content-type %q", r.Method, r.Header.Get("Content-Type"))
		}
		b, _ := io.ReadAll(r.Body)
		f.mu.Lock()
		f.body = string(b)
		f.mu.Unlock()
		if f.status != 0 {
			w.WriteHeader(f.status)
			return
		}
		u := f.srv.URL
		io.WriteString(w, "AP: 041-12345\nAU: "+u+"/dl/BaseSystem.dmg\nAH: AAAA\nAT: imgtok\n"+
			"CU: "+u+"/dl/BaseSystem.chunklist\nCH: BBBB\nCT: cnktok\n")
	case "/dl/BaseSystem.chunklist":
		io.WriteString(w, f.chunklist)
	case "/dl/BaseSystem.dmg":
		if f.shortDMG {
			w.Header().Set("Content-Length", "1000")
			io.WriteString(w, "short")
			return
		}
		w.Header().Set("Content-Length", strconv.Itoa(len(f.dmg)))
		io.WriteString(w, f.dmg)
	default:
		http.NotFound(w, r)
	}
}

func dirEntries(t *testing.T, dir string) []string {
	es, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, e := range es {
		names = append(names, e.Name())
	}
	return names
}

func TestRecoveryDownload(t *testing.T) {
	for _, tc := range []struct{ version, bid, os string }{
		{"sonoma", "Mac-827FAC58A8FDFA22", "default"},
		{"tahoe", "Mac-CFF7D910A743CAAF", "latest"},
	} {
		t.Run(tc.version, func(t *testing.T) {
			f := newFakeRecovery(t)
			dir := filepath.Join(t.TempDir(), "out")
			var mu sync.Mutex
			last := map[string]float64{}
			dmg, cnk, err := DownloadRecovery(context.Background(), tc.version, dir, func(s string, p float64) {
				mu.Lock()
				last[s] = p
				mu.Unlock()
			})
			if err != nil {
				t.Fatal(err)
			}
			if dmg != filepath.Join(dir, "BaseSystem.dmg") || cnk != filepath.Join(dir, "BaseSystem.chunklist") {
				t.Fatalf("paths %s %s", dmg, cnk)
			}
			if b, _ := os.ReadFile(dmg); string(b) != f.dmg {
				t.Error("dmg content mismatch")
			}
			if b, _ := os.ReadFile(cnk); string(b) != f.chunklist {
				t.Error("chunklist content mismatch")
			}
			if got := dirEntries(t, dir); len(got) != 2 {
				t.Errorf("dir entries %v", got)
			}
			if last["image"] != 100 || last["chunklist"] != 100 {
				t.Errorf("final progress %v", last)
			}

			re := regexp.MustCompile(`^cid=[0-9A-F]{16}\nsn=00000000000000000\nbid=` + tc.bid +
				`\nk=[0-9A-F]{64}\nfg=[0-9A-F]{64}\nos=` + tc.os + `$`)
			if !re.MatchString(f.body) {
				t.Errorf("POST body:\n%s", f.body)
			}
			want := map[string]string{
				"/InstallationPayload/RecoveryImage": "session=abc123",
				"/dl/BaseSystem.dmg":                 "AssetToken=imgtok",
				"/dl/BaseSystem.chunklist":           "AssetToken=cnktok",
			}
			for p, c := range want {
				if f.cookies[p] != c {
					t.Errorf("%s: Cookie %q, want %q", p, f.cookies[p], c)
				}
			}
		})
	}
}

func TestRecoveryUnknownVersion(t *testing.T) {
	if _, _, err := DownloadRecovery(context.Background(), "leopard", t.TempDir(), nil); err == nil || !strings.Contains(err.Error(), "unknown") {
		t.Fatalf("err = %v", err)
	}
}

func TestRecoveryForbidden(t *testing.T) {
	f := newFakeRecovery(t)
	f.status = http.StatusForbidden
	dir := t.TempDir()
	_, _, err := DownloadRecovery(context.Background(), "sonoma", dir, nil)
	if err == nil || !strings.Contains(err.Error(), "403") {
		t.Fatalf("err = %v", err)
	}
	if got := dirEntries(t, dir); len(got) != 0 {
		t.Errorf("leftover files %v", got)
	}
}

func TestRecoveryInterruptedLeavesNothing(t *testing.T) {
	f := newFakeRecovery(t)
	f.shortDMG = true
	dir := t.TempDir()
	if _, _, err := DownloadRecovery(context.Background(), "sonoma", dir, nil); err == nil {
		t.Fatal("expected error")
	}
	if got := dirEntries(t, dir); len(got) != 0 {
		t.Errorf("leftover files %v", got)
	}
}

func TestRecoveryCanceled(t *testing.T) {
	newFakeRecovery(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	dir := t.TempDir()
	if _, _, err := DownloadRecovery(ctx, "sonoma", dir, nil); err == nil {
		t.Fatal("expected error")
	}
	if got := dirEntries(t, dir); len(got) != 0 {
		t.Errorf("leftover files %v", got)
	}
}

// Names in the server's URLs must never reach the file system.
func TestRecoveryIgnoresServerFileNames(t *testing.T) {
	newFakeRecovery(t) // restores recoveryBaseURL on cleanup
	var u string // the fake server's own address; r.Host is osrecovery.apple.com by protocol
	evil := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/InstallationPayload/RecoveryImage" {
			io.WriteString(w, "AP: x\nAU: "+u+"/dl/%2e%2e\nAH: x\nAT: t\nCU: "+u+"/dl/%2e%2e%2f%2e%2e%2fvictim\nCH: x\nCT: t\n")
			return
		}
		if r.URL.Path == "/" {
			w.Header().Add("Set-Cookie", "session=abc")
		}
		io.WriteString(w, "payload")
	}))
	defer evil.Close()
	u = evil.URL
	recoveryBaseURL = evil.URL
	parent := t.TempDir()
	dir := filepath.Join(parent, "staging")
	dmg, cnk, err := DownloadRecovery(context.Background(), "sonoma", dir, nil)
	if err != nil {
		t.Fatal(err)
	}
	if dmg != filepath.Join(dir, "BaseSystem.dmg") || cnk != filepath.Join(dir, "BaseSystem.chunklist") {
		t.Fatalf("paths: %s %s", dmg, cnk)
	}
	if got := dirEntries(t, parent); len(got) != 1 {
		t.Fatalf("nothing may be written outside the staging dir: %v", got)
	}
}
