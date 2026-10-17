//go:build !reims_embed

package embed

import (
	"errors"
	"testing"
)

func TestStubIsUnavailable(t *testing.T) {
	if Available() {
		t.Fatal("the stub must not report embedding")
	}
	if _, err := NewEmbedder("", 1); !errors.Is(err, ErrUnavailable) {
		t.Fatalf("NewEmbedder: %v", err)
	}
}
