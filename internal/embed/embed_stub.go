//go:build !reims_embed

package embed

import "time"

// Available reports whether embedding is built in.
func Available() bool { return false }

// Probe reports whether an X server answers on display; always false without embedding.
func Probe(string) bool { return false }

// Embedder is unavailable without the reims_embed tag.
type Embedder struct{}

// NewEmbedder always fails with ErrUnavailable.
func NewEmbedder(string, uint32) (*Embedder, error) { return nil, ErrUnavailable }

func (*Embedder) FindWindow(uint32, Kind, time.Duration) (uint32, error) { return 0, ErrUnavailable }
func (*Embedder) Embed(uint32) (*Embedded, error)                        { return nil, ErrUnavailable }
func (*Embedder) Close()                                                 {}

// Embedded is unavailable without the reims_embed tag.
type Embedded struct{}

func (*Embedded) SetRect(int32, int32, uint32, uint32) error { return ErrUnavailable }
func (*Embedded) Pump() error                                { return ErrUnavailable }
func (*Embedded) Hide() error                                { return ErrUnavailable }
func (*Embedded) Show() error                                { return ErrUnavailable }
func (*Embedded) Focus() error                               { return ErrUnavailable }
func (*Embedded) Unfocus() error                             { return ErrUnavailable }
func (*Embedded) ReturnFocusOnAppClick() (bool, error)       { return false, ErrUnavailable }
func (*Embedded) Alive() bool                                { return false }
func (*Embedded) Release() error                             { return ErrUnavailable }
