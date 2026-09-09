package main

import (
	"context"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/coditary/dap/dap-agent/internal/config"
	"github.com/coditary/dap/dap-agent/internal/mcpbridge"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func main() {
	cfg := config.Parse()

	bridge := mcpbridge.New(cfg)
	mcpServer := mcp.NewServer(&mcp.Implementation{
		Name:    "dap-agent",
		Version: "0.2.0",
	}, &mcp.ServerOptions{
		Instructions: "DAP proxy MCP server. Talks to dap-cli over NDJSON repl. Use debug_sessions to discover sessions, debug_sync to refresh breakpoints, debug_threads/stack_trace/scopes/variables to inspect a stopped session, debug_navigate to step or continue, debug_stop to disconnect.",
	})
	bridge.Register(mcpServer)

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	if err := mcpServer.Run(ctx, &mcp.StdioTransport{}); err != nil {
		log.Fatalf("dap-agent: %v", err)
	}
	bridge.Close()
}
