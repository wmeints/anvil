package daemon

import (
	"sync"

	"github.com/wmeints/anvil/api/v1alpha1"
)

type sessionWriter struct {
	mu     sync.Mutex
	stream v1alpha1.AnvilService_AttachSandboxServer
}

func (w *sessionWriter) Write(p []byte) (int, error) {
	// Copy the data so we don't reuse existing structure that might get overwritten.
	outputData := append([]byte(nil), p...)

	outputMsg := &v1alpha1.AttachSandboxResponse_Stdout{
		Stdout: outputData,
	}

	responseData := &v1alpha1.AttachSandboxResponse{Msg: outputMsg}

	if err := w.send(responseData); err != nil {
		return 0, err
	}

	return len(p), nil
}

func (w *sessionWriter) send(resp *v1alpha1.AttachSandboxResponse) error {
	// Use the mutex to ensure only one sender is active at one time.
	w.mu.Lock()
	defer w.mu.Unlock()

	return w.stream.SendMsg(resp)
}
