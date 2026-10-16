// Recovery protocol ported from acidanthera's macrecovery (BSD-3-Clause, Copyright (c) 2019 vit9696)

package core

import (
	"context"
	"crypto/rand"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"strings"
	"time"
)

// RecoveryProduct identifies what Apple's recovery server should return for a macOS version.
type RecoveryProduct struct{ BoardID, MLB, OSType string }

// RecoveryProducts maps the short names used in config.go's Versions (high-sierra ... tahoe) to products,
// copied from macrecovery's product table (os_type defaults to "default").
var RecoveryProducts = map[string]RecoveryProduct{
	"high-sierra": {"Mac-7BA5B2D9E42DDD94", "00000000000J80300", "default"},
	"mojave":      {"Mac-7BA5B2DFE22DDD8C", "00000000000KXPG00", "default"},
	"catalina":    {"Mac-00BE6ED71E35EB86", "00000000000000000", "default"},
	"big-sur":     {"Mac-2BD1B31983FE1663", "00000000000000000", "default"},
	"monterey":    {"Mac-B809C3757DA9BB8D", "00000000000000000", "latest"},
	"ventura":     {"Mac-4B682C642B45593E", "00000000000000000", "latest"},
	"sonoma":      {"Mac-827FAC58A8FDFA22", "00000000000000000", "default"},
	"sequoia":     {"Mac-7BA5B2D9E42DDD94", "00000000000000000", "default"},
	"tahoe":       {"Mac-CFF7D910A743CAAF", "00000000000000000", "latest"},
}

// recoveryBaseURL is overridden by tests; the Host header stays osrecovery.apple.com either way.
var recoveryBaseURL = "http://osrecovery.apple.com"

const (
	recoveryHost      = "osrecovery.apple.com"
	recoveryUserAgent = "InternetRecovery/1.0"
)

// Keys macrecovery requires in the RecoveryImage response.
var recoveryRequiredKeys = []string{"AP", "AU", "AH", "AT", "CU", "CH", "CT"}

var (
	recoveryAPIClient = &http.Client{Timeout: 60 * time.Second}
	// No overall timeout: the DMG is large; ctx bounds it.
	recoveryDownloadClient = &http.Client{Transport: &http.Transport{
		Proxy:                 http.ProxyFromEnvironment,
		ResponseHeaderTimeout: 60 * time.Second,
		TLSHandshakeTimeout:   30 * time.Second,
	}}
)

// DownloadRecovery downloads the chunklist and the DMG for version into dir and returns their paths.
// progress(stage, pct) is called with stage "chunklist" or "image" and pct in 0..100, or pct=-1 when the size is unknown; it may be nil.
func DownloadRecovery(ctx context.Context, version, dir string, progress func(stage string, pct float64)) (dmgPath, chunklistPath string, err error) {
	p, ok := RecoveryProducts[version]
	if !ok {
		return "", "", fmt.Errorf("recovery: unknown macOS version %q", version)
	}
	session, err := recoverySession(ctx)
	if err != nil {
		return "", "", err
	}
	info, err := recoveryImageInfo(ctx, session, p)
	if err != nil {
		return "", "", err
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return "", "", err
	}
	chunklistPath, err = recoverySave(ctx, info["CU"], info["CT"], dir, "chunklist", progress)
	if err != nil {
		return "", "", err
	}
	dmgPath, err = recoverySave(ctx, info["AU"], info["AT"], dir, "image", progress)
	if err != nil {
		os.Remove(chunklistPath)
		return "", "", err
	}
	return dmgPath, chunklistPath, nil
}

func newRecoveryRequest(ctx context.Context, method, rawURL, host string, body io.Reader) (*http.Request, error) {
	req, err := http.NewRequestWithContext(ctx, method, rawURL, body)
	if err != nil {
		return nil, err
	}
	req.Host = host
	req.Close = true // Connection: close
	req.Header.Set("User-Agent", recoveryUserAgent)
	// urllib sends identity; it also stops Go from transparently gunzipping the image.
	req.Header.Set("Accept-Encoding", "identity")
	return req, nil
}

// recoverySession fetches the "session=..." cookie from the server root.
func recoverySession(ctx context.Context) (string, error) {
	req, err := newRecoveryRequest(ctx, http.MethodGet, recoveryBaseURL+"/", recoveryHost, nil)
	if err != nil {
		return "", err
	}
	resp, err := recoveryAPIClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("recovery: session: %w", err)
	}
	defer resp.Body.Close()
	io.Copy(io.Discard, resp.Body)
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("recovery: session: unexpected status %s", resp.Status)
	}
	for _, h := range resp.Header.Values("Set-Cookie") {
		for _, c := range strings.Split(h, "; ") {
			if strings.HasPrefix(c, "session=") {
				return c, nil
			}
		}
	}
	return "", errors.New("recovery: no session cookie in server response")
}

// recoveryImageInfo POSTs the board identity and parses the "key: value" reply.
func recoveryImageInfo(ctx context.Context, session string, p RecoveryProduct) (map[string]string, error) {
	cid, err := recoveryID(16)
	if err != nil {
		return nil, err
	}
	k, err := recoveryID(64)
	if err != nil {
		return nil, err
	}
	fg, err := recoveryID(64)
	if err != nil {
		return nil, err
	}
	body := strings.Join([]string{
		"cid=" + cid,
		"sn=" + p.MLB,
		"bid=" + p.BoardID,
		"k=" + k,
		"fg=" + fg,
		"os=" + p.OSType,
	}, "\n")
	req, err := newRecoveryRequest(ctx, http.MethodPost, recoveryBaseURL+"/InstallationPayload/RecoveryImage", recoveryHost, strings.NewReader(body))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Cookie", session)
	req.Header.Set("Content-Type", "text/plain")
	resp, err := recoveryAPIClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("recovery: image info: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("recovery: image info: unexpected status %s", resp.Status)
	}
	out, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return nil, fmt.Errorf("recovery: image info: %w", err)
	}
	info := map[string]string{}
	for _, line := range strings.Split(string(out), "\n") {
		// Like Python's line.split(': '): only lines with exactly one separator count.
		if kv := strings.Split(line, ": "); len(kv) == 2 {
			info[kv[0]] = kv[1]
		}
	}
	for _, key := range recoveryRequiredKeys {
		if _, ok := info[key]; !ok {
			return nil, fmt.Errorf("recovery: server response missing key %s", key)
		}
	}
	return info, nil
}

// recoveryID returns n random uppercase hex digits.
func recoveryID(n int) (string, error) {
	const digits = "0123456789ABCDEF"
	b := make([]byte, n)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	for i := range b {
		b[i] = digits[b[i]&0xF]
	}
	return string(b), nil
}

// recoverySave downloads rawURL into dir/<basename of URL path> via a .part file.
func recoverySave(ctx context.Context, rawURL, token, dir, stage string, progress func(string, float64)) (dst string, err error) {
	u, err := url.Parse(rawURL)
	if err != nil {
		return "", fmt.Errorf("recovery: bad %s URL: %w", stage, err)
	}
	name := path.Base(u.Path)
	if name == "" || name == "." || name == "/" || strings.ContainsAny(name, `/\`) {
		return "", fmt.Errorf("recovery: invalid save name from %s", rawURL)
	}
	dst = filepath.Join(dir, name)

	req, err := newRecoveryRequest(ctx, http.MethodGet, rawURL, u.Hostname(), nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("Cookie", "AssetToken="+token)
	resp, err := recoveryDownloadClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("recovery: download %s: %w", name, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("recovery: download %s: unexpected status %s", name, resp.Status)
	}

	part := dst + ".part"
	f, err := os.Create(part)
	if err != nil {
		return "", err
	}
	defer func() {
		if err != nil {
			f.Close()
			os.Remove(part)
		}
	}()

	total := resp.ContentLength
	report := func(n int64) {
		if progress == nil {
			return
		}
		if total > 0 {
			progress(stage, float64(n)*100/float64(total))
		} else {
			progress(stage, -1)
		}
	}
	report(0)
	var n int64
	buf := make([]byte, 1<<20)
	for {
		r, rerr := resp.Body.Read(buf)
		if r > 0 {
			if _, werr := f.Write(buf[:r]); werr != nil {
				return "", werr
			}
			n += int64(r)
			report(n)
		}
		if rerr == io.EOF {
			break
		}
		if rerr != nil {
			return "", fmt.Errorf("recovery: download %s: %w", name, rerr)
		}
	}
	if total > 0 && n != total {
		return "", fmt.Errorf("recovery: download %s: got %d of %d bytes", name, n, total)
	}
	if err = f.Close(); err != nil {
		return "", err
	}
	if err = os.Rename(part, dst); err != nil {
		return "", err
	}
	return dst, nil
}
