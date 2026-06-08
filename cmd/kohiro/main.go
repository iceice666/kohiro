package main

import (
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"
	"strings"
	"syscall"

	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"
	wishgit "github.com/charmbracelet/wish/git"
	"github.com/charmbracelet/wish/logging"
	gossh "golang.org/x/crypto/ssh"

	"github.com/iceice666/kohiro/auth"
	kohirogit "github.com/iceice666/kohiro/git"
	"github.com/iceice666/kohiro/store"
)

const (
	listenAddr = "0.0.0.0:2222"
	hostKeyDir = "./data/.ssh"
	dbPath     = "./data/kohiro.db"
)

func main() {
	adminKeyFile := flag.String("admin-key", "", "path to admin public key (.pub) for bootstrap")
	adminUser := flag.String("admin-user", "admin", "username for the bootstrap admin")
	setPublic := flag.String("set-public", "", "mark a repo public: owner/name (opens DB, sets flag, exits)")
	setPrivate := flag.String("set-private", "", "mark a repo private: owner/name (opens DB, sets flag, exits)")
	flag.Parse()

	if err := os.MkdirAll(hostKeyDir, 0o700); err != nil {
		log.Fatal(err)
	}
	if err := os.MkdirAll(kohirogit.RepoDir, 0o700); err != nil {
		log.Fatal(err)
	}

	st, err := store.Open(dbPath)
	if err != nil {
		log.Fatalf("open store: %v", err)
	}
	defer st.Close()

	if *adminKeyFile != "" {
		if err := bootstrapAdmin(st, *adminUser, *adminKeyFile); err != nil {
			log.Fatalf("bootstrap admin: %v", err)
		}
	}

	if *setPublic != "" {
		owner, name, ok := splitOwnerName(*setPublic)
		if !ok {
			log.Fatalf("--set-public: expected owner/name, got %q", *setPublic)
		}
		if err := st.SetPublic(owner, name, true); err != nil {
			log.Fatalf("set-public %s: %v", *setPublic, err)
		}
		log.Printf("marked %s public", *setPublic)
		return
	}
	if *setPrivate != "" {
		owner, name, ok := splitOwnerName(*setPrivate)
		if !ok {
			log.Fatalf("--set-private: expected owner/name, got %q", *setPrivate)
		}
		if err := st.SetPublic(owner, name, false); err != nil {
			log.Fatalf("set-private %s: %v", *setPrivate, err)
		}
		log.Printf("marked %s private", *setPrivate)
		return
	}

	hooks := auth.New(st)

	s, err := wish.NewServer(
		wish.WithAddress(listenAddr),
		wish.WithHostKeyPath(hostKeyDir+"/host_key"),
		// Accept all keys at the SSH layer; AuthRepo enforces per-repo access.
		wish.WithPublicKeyAuth(func(_ ssh.Context, _ ssh.PublicKey) bool {
			return true
		}),
		wish.WithMiddleware(
			greetMiddleware,
			wishgit.Middleware(kohirogit.RepoDir, hooks),
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

func splitOwnerName(s string) (owner, name string, ok bool) {
	idx := strings.IndexByte(s, '/')
	if idx < 0 || idx == len(s)-1 {
		return "", "", false
	}
	return s[:idx], s[idx+1:], true
}

func bootstrapAdmin(st *store.Store, username, keyFile string) error {
	data, err := os.ReadFile(keyFile)
	if err != nil {
		return err
	}
	pk, comment, _, _, err := gossh.ParseAuthorizedKey(data)
	if err != nil {
		return err
	}
	if comment == "" {
		comment = username
	}
	fp := gossh.FingerprintSHA256(pk)
	return st.Bootstrap(username, fp, comment)
}
