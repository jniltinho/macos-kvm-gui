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

var ErrQMPClosed = errors.New("conexão QMP encerrada")

// QMPEvent é um evento assíncrono do QEMU (STOP, RESUME, BLOCK_IO_ERROR, ...).
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

// QMP é um cliente mínimo: uma conexão, um comando por vez, eventos por callback.
type QMP struct {
	conn    net.Conn
	mu      sync.Mutex // serializa comandos
	replies chan qmpMsg
	done    chan struct{}
}

// DialQMP espera o socket aparecer, negocia capacidades e passa a entregar eventos.
// onDisconnect é chamado uma vez quando a conexão cai.
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
		return nil, fmt.Errorf("QMP indisponível: %w", err)
	}
	dec := json.NewDecoder(conn)
	conn.SetDeadline(time.Now().Add(qmpTimeout))
	var greeting struct {
		QMP json.RawMessage `json:"QMP"`
	}
	if err := dec.Decode(&greeting); err != nil || greeting.QMP == nil {
		conn.Close()
		return nil, fmt.Errorf("greeting QMP inválido: %v", err)
	}
	if _, err := conn.Write([]byte(`{"execute":"qmp_capabilities"}` + "\n")); err != nil {
		conn.Close()
		return nil, err
	}
	for {
		var m qmpMsg
		if err := dec.Decode(&m); err != nil {
			conn.Close()
			return nil, fmt.Errorf("negociação QMP falhou: %w", err)
		}
		if m.Error != nil {
			conn.Close()
			return nil, fmt.Errorf("negociação QMP recusada: %s", m.Error.Desc)
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
			default: // resposta sem ninguém esperando (comando que já deu timeout)
			}
		}
	}()
	return q, nil
}

// Execute envia um comando e espera a resposta, com prazo de 5 s.
func (q *QMP) Execute(cmd string) error {
	q.mu.Lock()
	defer q.mu.Unlock()
	select { // descarta resposta atrasada de um comando anterior
	case <-q.replies:
	default:
	}
	data, _ := json.Marshal(map[string]string{"execute": cmd})
	q.conn.SetWriteDeadline(time.Now().Add(qmpTimeout))
	if _, err := q.conn.Write(append(data, '\n')); err != nil {
		return ErrQMPClosed
	}
	select {
	case m := <-q.replies:
		if m.Error != nil {
			return fmt.Errorf("%s: %s", cmd, m.Error.Desc)
		}
		return nil
	case <-q.done:
		return ErrQMPClosed
	case <-time.After(qmpTimeout):
		return fmt.Errorf("%s: sem resposta do QEMU em %s", cmd, qmpTimeout)
	}
}

func (q *QMP) Close() error { return q.conn.Close() }
