package main

import (
	"fmt"
	"log"
	"net/http"
	"os"

	"github.com/quickfixgo/quickfix"
)

type FIXApplication struct {
	SessionIDs map[string]quickfix.SessionID
}

func startWebServer() {
    go func() {
    fs := http.FileServer(http.Dir("."))
    http.Handle("/", http.StripPrefix("/", fs))

    log.Printf("Server starting on http://%s", ":8080")
    if err := http.ListenAndServe(":8080", nil); err != nil {
        log.Fatalf("Error starting server: %s", err)
    }
}()
}

func (a *FIXApplication) OnLogon(sessionID quickfix.SessionID){
	fmt.Printf("Session has logged on.\n")
}

func (a *FIXApplication) OnLogout(sessionID quickfix.SessionID){
	fmt.Printf("Logout.")
}

func (a *FIXApplication) ToAdmin(msg *quickfix.Message, sessionID quickfix.SessionID) {
	////Notification of admin message being sent to target.
	msgType, err := msg.MsgType()
	if err == nil && msgType == "A" { // Check if it's a Logon message
		// Add or override fields for Logon message
		fmt.Printf("logon")
	}
}

func (a *FIXApplication) OnCreate(sessionID quickfix.SessionID){
	a.SessionIDs[sessionID.String()] = sessionID
}

func (a *FIXApplication) FromAdmin(msg *quickfix.Message, sessionID quickfix.SessionID) (reject quickfix.MessageRejectError) {
	//Notification of admin message being received from target.
	return
}

func (a *FIXApplication) ToApp(msg *quickfix.Message, sessionID quickfix.SessionID) (err error) {
	//Notification of app message being sent to target.
	return
}

func (a *FIXApplication) FromApp(msg *quickfix.Message, sessionID quickfix.SessionID) (reject quickfix.MessageRejectError) {
	//handles application-level messages that are not recognized by QuickFIX/n as administrative or session-level messages.
    return
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

type screenLogFactory struct{
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

	startWebServer()

	fixApp := &FIXApplication{
		SessionIDs: make(map[string]quickfix.SessionID),
	}

	//logFactory := quickfix.NewScreenLogFactory()
	logFactory := NewFancyLog("./logfile.log")

	cfg, err := os.Open("sessions.cfg")
	if err != nil {
		fmt.Printf("Error opening config: %v", err)
		return
	}

	appSettings, err := quickfix.ParseSettings(cfg)
	//fmt.Println(appSettings)
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

