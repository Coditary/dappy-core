package repl

import "testing"

func TestTrimLine(t *testing.T) {
	if got := string(trimLine([]byte("{\"ok\":true}\n"))); got != "{\"ok\":true}" {
		t.Fatalf("unexpected trim: %q", got)
	}
	if got := string(trimLine([]byte("\r\n"))); got != "" {
		t.Fatalf("expected empty, got %q", got)
	}
}
