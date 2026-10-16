package core

import (
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"sync"
	"time"
)

const (
	qmpTimeout     = 5 * time.Second
	qmpDialTries   = 50
	qmpDialBackoff = 100 * time.Millisecond
)

var ErrQMPClosed = errors.New("QMP connection closed")

// QMPEvent is an asynchronous QEMU event (STOP, RESUME, BLOCK_IO_ERROR, ...).
type QMPEvent struct {
	Name string         `json:"event"`
	Data map[string]any `json:"data"`
}

type qmpMsg struct {
	Event  string          `json:"event"`
	Data   map[string]any  `json:"data"`
	Return json.RawMessage `json:"return"`
	Error  *struct {
		Class string `json:"class"`
		Desc  string `json:"desc"`
	} `json:"error"`
}

// QMP is a minimal client: one connection, one command at a time, events via callback.
type QMP struct {
	conn    net.Conn
	mu      sync.Mutex // serializes commands
	replies chan qmpMsg
	done    chan struct{}
}

// DialQMP waits for the socket to appear, negotiates capabilities and starts delivering events.
// onDisconnect is called once when the connection drops.
func DialQMP(path string, onEvent func(QMPEvent), onDisconnect func()) (*QMP, error) {
	var conn net.Conn
	var err error
	for range qmpDialTries {
		if conn, err = net.Dial("unix", path); err == nil {
			break
		}
		time.Sleep(qmpDialBackoff)
	}
	if err != nil {
		return nil, fmt.Errorf("QMP unavailable: %w", err)
	}
	dec := json.NewDecoder(conn)
	conn.SetDeadline(time.Now().Add(qmpTimeout))
	var greeting struct {
		QMP json.RawMessage `json:"QMP"`
	}
	if err := dec.Decode(&greeting); err != nil || greeting.QMP == nil {
		conn.Close()
		return nil, fmt.Errorf("invalid QMP greeting: %v", err)
	}
	if _, err := conn.Write([]byte(`{"execute":"qmp_capabilities"}` + "\n")); err != nil {
		conn.Close()
		return nil, err
	}
	for {
		var m qmpMsg
		if err := dec.Decode(&m); err != nil {
			conn.Close()
			return nil, fmt.Errorf("QMP negotiation failed: %w", err)
		}
		if m.Error != nil {
			conn.Close()
			return nil, fmt.Errorf("QMP negotiation refused: %s", m.Error.Desc)
		}
		if m.Return != nil {
			break
		}
	}
	conn.SetDeadline(time.Time{})

	q := &QMP{conn: conn, replies: make(chan qmpMsg, 1), done: make(chan struct{})}
	go func() {
		defer func() {
			close(q.done)
			if onDisconnect != nil {
				onDisconnect()
			}
		}()
		for {
			var m qmpMsg
			if err := dec.Decode(&m); err != nil {
				return
			}
			if m.Event != "" {
				if onEvent != nil {
					onEvent(QMPEvent{m.Event, m.Data})
				}
				continue
			}
			select {
			case q.replies <- m:
			default: // reply nobody is waiting for (command that already timed out)
			}
		}
	}()
	return q, nil
}

// Execute sends a command and waits for the reply, with a 5 s deadline.
func (q *QMP) Execute(cmd string) error {
	_, err := q.Query(cmd)
	return err
}

// Query sends a command and returns its reply's "return" value.
func (q *QMP) Query(cmd string) (json.RawMessage, error) {
	q.mu.Lock()
	defer q.mu.Unlock()
	select { // drop a late reply from a previous command
	case <-q.replies:
	default:
	}
	data, _ := json.Marshal(map[string]string{"execute": cmd})
	q.conn.SetWriteDeadline(time.Now().Add(qmpTimeout))
	if _, err := q.conn.Write(append(data, '\n')); err != nil {
		return nil, ErrQMPClosed
	}
	select {
	case m := <-q.replies:
		if m.Error != nil {
			return nil, fmt.Errorf("%s: %s", cmd, m.Error.Desc)
		}
		return m.Return, nil
	case <-q.done:
		return nil, ErrQMPClosed
	case <-time.After(qmpTimeout):
		return nil, fmt.Errorf("%s: no reply from QEMU within %s", cmd, qmpTimeout)
	}
}

func (q *QMP) Close() error { return q.conn.Close() }
