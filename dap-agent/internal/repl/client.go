package repl

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"sync"
	"sync/atomic"
)

// Options configure the long-lived dap-cli NDJSON repl subprocess.
type Options struct {
	CLIPath     string
	ControlPort uint
	Scope       string
	Program     string
	Adapter     string
	Env         []string
}

// Client talks to `dap-cli debug repl --ndjson` over stdin/stdout.
type Client struct {
	opts   Options
	cmd    *exec.Cmd
	stdin  io.WriteCloser
	stdout *bufio.Reader
	mu     sync.Mutex
	seq    atomic.Int64
	closed bool
}

// Response is the NDJSON envelope from dap-cli repl.
type Response struct {
	ID     json.RawMessage `json:"id"`
	OK     bool            `json:"ok"`
	Result json.RawMessage `json:"result"`
	Error  string          `json:"error"`
}

// Start launches dap-cli and consumes the initial ready event.
func Start(ctx context.Context, opts Options) (*Client, error) {
	if opts.CLIPath == "" {
		return nil, errors.New("cli path is required")
	}

	args := []string{"debug", "repl", "--ndjson"}
	if opts.ControlPort > 0 {
		args = append(args, "--control-port", fmt.Sprintf("%d", opts.ControlPort))
	}
	if opts.Scope != "" {
		args = append(args, "--scope", opts.Scope)
	}
	if opts.Adapter != "" {
		args = append(args, "--adapter", opts.Adapter)
	}
	if opts.Program != "" {
		args = append(args, opts.Program)
	}

	cmd := exec.CommandContext(ctx, opts.CLIPath, args...)
	cmd.Env = append(os.Environ(), opts.Env...)
	cmd.Stderr = os.Stderr

	stdin, err := cmd.StdinPipe()
	if err != nil {
		return nil, fmt.Errorf("stdin pipe: %w", err)
	}
	stdoutPipe, err := cmd.StdoutPipe()
	if err != nil {
		stdin.Close()
		return nil, fmt.Errorf("stdout pipe: %w", err)
	}
	if err := cmd.Start(); err != nil {
		stdin.Close()
		return nil, fmt.Errorf("start dap-cli: %w", err)
	}

	client := &Client{
		opts:   opts,
		cmd:    cmd,
		stdin:  stdin,
		stdout: bufio.NewReader(stdoutPipe),
	}

	ready, err := client.readResponse(ctx)
	if err != nil {
		client.Close()
		return nil, fmt.Errorf("read ready event: %w", err)
	}
	if !ready.OK {
		return nil, fmt.Errorf("repl ready failed: %s", ready.Error)
	}
	return client, nil
}

// Op sends a repl NDJSON operation and returns the parsed result object.
func (c *Client) Op(ctx context.Context, op string, fields map[string]any) (json.RawMessage, error) {
	req := map[string]any{"op": op}
	for k, v := range fields {
		req[k] = v
	}
	resp, err := c.Call(ctx, req)
	if err != nil {
		return nil, err
	}
	return resp.Result, nil
}

// Call sends a request with an auto-incrementing id.
func (c *Client) Call(ctx context.Context, req map[string]any) (*Response, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return nil, errors.New("repl client is closed")
	}

	id := c.seq.Add(1)
	req["id"] = id

	line, err := json.Marshal(req)
	if err != nil {
		return nil, err
	}
	line = append(line, '\n')

	if err := writeAll(ctx, c.stdin, line); err != nil {
		return nil, err
	}

	resp, err := c.readResponse(ctx)
	if err != nil {
		return nil, err
	}
	if !resp.OK {
		if resp.Error != "" {
			return resp, fmt.Errorf("%s", resp.Error)
		}
		return resp, errors.New("repl request failed")
	}
	return resp, nil
}

// Close terminates the repl subprocess.
func (c *Client) Close() error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return nil
	}
	c.closed = true
	_ = c.stdin.Close()
	if c.cmd != nil && c.cmd.Process != nil {
		_ = c.cmd.Process.Kill()
	}
	if c.cmd != nil {
		_ = c.cmd.Wait()
	}
	return nil
}

func (c *Client) readResponse(ctx context.Context) (*Response, error) {
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		line, err := c.stdout.ReadBytes('\n')
		if err != nil {
			return nil, fmt.Errorf("read repl response: %w", err)
		}
		trimmed := trimLine(line)
		if len(trimmed) == 0 {
			continue
		}
		var resp Response
		if err := json.Unmarshal(trimmed, &resp); err != nil {
			return nil, fmt.Errorf("decode repl response: %w", err)
		}
		return &resp, nil
	}
}

func trimLine(line []byte) []byte {
	for len(line) > 0 && (line[len(line)-1] == '\n' || line[len(line)-1] == '\r') {
		line = line[:len(line)-1]
	}
	return line
}

func writeAll(ctx context.Context, w io.Writer, data []byte) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	_, err := w.Write(data)
	return err
}

// RunCLI executes a one-shot dap-cli command and returns stdout.
func RunCLI(ctx context.Context, cliPath string, args []string, env []string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, cliPath, args...)
	cmd.Env = append(os.Environ(), env...)
	out, err := cmd.Output()
	if err != nil {
		if exit, ok := err.(*exec.ExitError); ok {
			return nil, fmt.Errorf("dap-cli %v failed: %s", args, string(exit.Stderr))
		}
		return nil, fmt.Errorf("dap-cli %v: %w", args, err)
	}
	return out, nil
}
