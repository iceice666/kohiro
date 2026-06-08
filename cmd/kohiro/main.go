package main

import (
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"
	wishgit "github.com/charmbracelet/wish/git"
	"github.com/charmbracelet/wish/logging"

	kohirogit "github.com/iceice666/kohiro/git"
)

const (
	listenAddr = "0.0.0.0:2222"
	hostKeyDir = "./data/.ssh"
)

func main() {
	if err := os.MkdirAll(hostKeyDir, 0o700); err != nil {
		log.Fatal(err)
	}
	if err := os.MkdirAll(kohirogit.RepoDir, 0o700); err != nil {
		log.Fatal(err)
	}

	s, err := wish.NewServer(
		wish.WithAddress(listenAddr),
		wish.WithHostKeyPath(hostKeyDir+"/host_key"),
		wish.WithPublicKeyAuth(func(_ ssh.Context, _ ssh.PublicKey) bool {
			return true // accept all keys; real auth comes next
		}),
		wish.WithMiddleware(
			greetMiddleware,
			wishgit.Middleware(kohirogit.RepoDir, allowAllHooks{}),
			logging.Middleware(),
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

// allowAllHooks grants read-write access to every repo for every key.
// Real auth is wired in Milestone 3.
type allowAllHooks struct{}

func (allowAllHooks) AuthRepo(_ string, _ ssh.PublicKey) wishgit.AccessLevel {
	return wishgit.ReadWriteAccess
}

func (allowAllHooks) Push(repo string, _ ssh.PublicKey) {
	// TODO milestone 5: enqueue CI run; note: refs are NOT on stdin here —
	// query them via go-git or drop a hooks/post-receive file into the repo at Init time.
	log.Printf("post-receive: %s", repo)
}

func (allowAllHooks) Fetch(_ string, _ ssh.PublicKey) {}
