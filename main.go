package main

import (
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"sync"

	"github.com/quickfixgo/quickfix"
)

type FIXApplication struct {
	SessionStatus map[string]bool
	mu            sync.RWMutex
}

// OnCreate is called when a new session is created.
func (a *FIXApplication) OnCreate(sessionID quickfix.SessionID) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.SessionStatus[sessionID.String()] = false // Initially not connected
	fmt.Printf("Session %s created.\n", sessionID)
}

// OnLogon updates the session status when a logon occurs.
func (a *FIXApplication) OnLogon(sessionID quickfix.SessionID) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.SessionStatus[sessionID.String()] = true
	fmt.Printf("Session %s has logged on.\n", sessionID)
}

// OnLogout updates the session status when a logout occurs.
func (a *FIXApplication) OnLogout(sessionID quickfix.SessionID) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.SessionStatus[sessionID.String()] = false
	fmt.Printf("Session %s has logged out.\n", sessionID)
}

// ToAdmin is called when an admin message is being sent.
func (a *FIXApplication) ToAdmin(msg *quickfix.Message, sessionID quickfix.SessionID) {
	msgType, err := msg.MsgType()
	if err == nil && msgType == "A" { // Check if it's a Logon message
		fmt.Printf("Logon message sent for session %s\n", sessionID)
	}
}

// FromAdmin is called when an admin message is received.
func (a *FIXApplication) FromAdmin(msg *quickfix.Message, sessionID quickfix.SessionID) (reject quickfix.MessageRejectError) {
	return
}

// ToApp is called when an app message is being sent.
func (a *FIXApplication) ToApp(msg *quickfix.Message, sessionID quickfix.SessionID) (err error) {
	return
}

// FromApp is called when an app message is received.
func (a *FIXApplication) FromApp(msg *quickfix.Message, sessionID quickfix.SessionID) (reject quickfix.MessageRejectError) {
	return
}

func startWebServer(fixApp *FIXApplication) {
	http.HandleFunc("/sessions", func(w http.ResponseWriter, r *http.Request) {
		fixApp.mu.RLock()
		defer fixApp.mu.RUnlock()

		sessionDetails := []map[string]string{}
		for sessionID, connected := range fixApp.SessionStatus {
			status := "Disconnected"
			if connected {
				status = "Connected"
			}
			sessionDetail := map[string]string{
				"SessionID": sessionID,
				"Status":    status,
			}
			sessionDetails = append(sessionDetails, sessionDetail)
		}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(sessionDetails)
	})

	go func() {
		fs := http.FileServer(http.Dir("."))
		http.Handle("/", http.StripPrefix("/", fs))

		log.Printf("Server starting on http://%s", ":8080")
		if err := http.ListenAndServe(":8080", nil); err != nil {
			log.Fatalf("Error starting server: %s", err)
		}
	}()
}

type screenLog struct {
	prefix string
	file   *os.File
}

func (l screenLog) OnIncoming(s []byte) {
	fmt.Fprintf(l.file, "<=== Incoming FIX Msg: <===\n%s\n", string(s))
}

func (l screenLog) OnOutgoing(s []byte) {
	fmt.Fprintf(l.file, "===> Outgoing FIX Msg: ===>\n%s\n", string(s))
}

func (l screenLog) OnEvent(s string) {
	fmt.Fprintf(l.file, "==== Event: ====\n%s\n", s)
}

func (l screenLog) OnEventf(format string, a ...interface{}) {
	l.OnEvent(fmt.Sprintf(format, a...))
}

type screenLogFactory struct {
	filePath string
}

func (f screenLogFactory) Create() (quickfix.Log, error) {
	file, err := os.OpenFile(f.filePath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
	if err != nil {
		return nil, err
	}
	log := screenLog{"GLOBAL", file}
	return log, nil
}

func (f screenLogFactory) CreateSessionLog(sessionID quickfix.SessionID) (quickfix.Log, error) {
	file, err := os.OpenFile(f.filePath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
	if err != nil {
		return nil, err
	}
	log := screenLog{sessionID.String(), file}
	return log, nil
}

// NewFancyLog creates an instance of LogFactory that writes messages and events to stdout.
func NewFancyLog(filePath string) quickfix.LogFactory {
	return screenLogFactory{filePath}
}

func main() {
	fixApp := &FIXApplication{
		SessionStatus: make(map[string]bool),
	}

	startWebServer(fixApp)

	logFactory := NewFancyLog("./logfile.log")

	cfg, err := os.Open("sessions.cfg")
	if err != nil {
		fmt.Printf("Error opening config: %v", err)
		return
	}

	appSettings, err := quickfix.ParseSettings(cfg)
	if err != nil {
		fmt.Println("Error reading cfg,", err)
		return
	}

	initiator, err := quickfix.NewInitiator(fixApp, quickfix.NewMemoryStoreFactory(), appSettings, logFactory)
	if err != nil {
		log.Fatalf("Unable to create Initiator: %s\n", err)
	}

	if err = initiator.Start(); err != nil {
		log.Fatal(err)
	}
	defer initiator.Stop()

	select {}
}
