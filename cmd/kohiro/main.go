package main

import (
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"
	"github.com/charmbracelet/wish/logging"
)

const (
	listenAddr = "0.0.0.0:2222"
	hostKeyDir = "./data/.ssh"
)

func main() {
	if err := os.MkdirAll(hostKeyDir, 0o700); err != nil {
		log.Fatal(err)
	}

	s, err := wish.NewServer(
		wish.WithAddress(listenAddr),
		wish.WithHostKeyPath(hostKeyDir+"/host_key"),
		wish.WithPublicKeyAuth(func(_ ssh.Context, _ ssh.PublicKey) bool {
			return true // accept all keys; real auth comes next
		}),
		wish.WithMiddleware(
			logging.Middleware(),
			greetMiddleware,
		),
	)
	if err != nil {
		log.Fatal(err)
	}

	sig := make(chan os.Signal, 1)
	signal.Notify(sig, os.Interrupt, syscall.SIGTERM)

	go func() {
		log.Printf("kohiro listening on %s", listenAddr)
		if err := s.ListenAndServe(); err != nil {
			log.Printf("server stopped: %v", err)
		}
	}()

	<-sig
	_ = s.Close()
}

func greetMiddleware(next ssh.Handler) ssh.Handler {
	return func(sess ssh.Session) {
		fmt.Fprintf(sess, "hello, %s — kohiro is alive\n", sess.User())
		next(sess)
	}
}
