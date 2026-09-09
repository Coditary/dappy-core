package config

import (
	"flag"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
)

// Config holds runtime options for the MCP server.
type Config struct {
	CLIPath     string
	ControlPort uint
	Scope       string
	Program     string
	Adapter     string
}

func Parse() Config {
	var cfg Config
	flag.StringVar(&cfg.CLIPath, "cli", envOr("DAP_CLI", ""), "path to dap-cli binary (default: DAP_CLI, PATH, or next to dap-agent)")
	var controlPort int
	flag.IntVar(&controlPort, "control-port", envIntOr("DAP_CONTROL_PORT", 0), "control attach port for dap-cli repl")
	flag.StringVar(&cfg.Scope, "scope", envOr("DAP_SCOPE_ID", ""), "session scope filter (DAP_SCOPE_ID)")
	flag.StringVar(&cfg.Program, "program", envOr("DAP_PROGRAM", ""), "optional program for an owned repl session")
	flag.StringVar(&cfg.Adapter, "adapter", envOr("DAP_ADAPTER", ""), "optional adapter id for an owned repl session")
	flag.Parse()
	if controlPort < 0 {
		controlPort = 0
	}
	cfg.ControlPort = uint(controlPort)
	cfg.CLIPath = resolveCLI(cfg.CLIPath)
	return cfg
}

func resolveCLI(explicit string) string {
	if explicit != "" {
		return explicit
	}
	if path, err := exec.LookPath("dap-cli"); err == nil {
		return path
	}
	if self, err := os.Executable(); err == nil {
		dir := filepath.Dir(self)
		for _, name := range []string{"dap-cli", filepath.Join("..", "dap-cli")} {
			candidate := filepath.Join(dir, name)
			if info, err := os.Stat(candidate); err == nil && !info.IsDir() {
				return candidate
			}
		}
	}
	return "dap-cli"
}

func envOr(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func envIntOr(key string, fallback int) int {
	v := os.Getenv(key)
	if v == "" {
		return fallback
	}
	n, err := strconv.Atoi(v)
	if err != nil {
		return fallback
	}
	return n
}
