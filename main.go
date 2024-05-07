package main

import (
	"fmt"
	"log"
	"os"
	"time"

	"github.com/fatih/color"
	"github.com/gosuri/uitable"

	"github.com/quickfixgo/quickfix"
)

type FIXApplication struct {
	SessionIDs map[string]quickfix.SessionID
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
}

func (l screenLog) OnIncoming(s []byte) {
	table := uitable.New()
	table.MaxColWidth = 150
	table.Wrap = true // wrap columns

	table.AddRow(" |Time:", fmt.Sprintf("%v", time.Now().UTC()))
	table.AddRow(" |Session:", l.prefix)
	table.AddRow(" |Content:", string(s))

	color.Set(color.Bold, color.FgBlue)
	fmt.Println("<=== Incoming FIX Msg: <===")
	fmt.Println(table)
	color.Unset()
}

func (l screenLog) OnOutgoing(s []byte) {
	table := uitable.New()
	table.MaxColWidth = 150
	table.Wrap = true // wrap columns

	table.AddRow(" |Time:", fmt.Sprintf("%v", time.Now().UTC()))
	table.AddRow(" |Session:", l.prefix)
	table.AddRow(" |Content:", string(s))

	color.Set(color.Bold, color.FgMagenta)
	fmt.Println("===> Outgoing FIX Msg: ===>")
	fmt.Println(table)
	color.Unset()
}

func (l screenLog) OnEvent(s string) {

	table := uitable.New()
	table.MaxColWidth = 150
	table.Wrap = true // wrap columns

	table.AddRow(" |Time:", fmt.Sprintf("%v", time.Now().UTC()))
	table.AddRow(" |Session:", l.prefix)
	table.AddRow(" |Content:", s)

	color.Set(color.Bold, color.FgCyan)
	fmt.Println("==== Event: ====")
	fmt.Println(table)
	color.Unset()
}

func (l screenLog) OnEventf(format string, a ...interface{}) {
	l.OnEvent(fmt.Sprintf(format, a...))
}

type screenLogFactory struct{}

func (screenLogFactory) Create() (quickfix.Log, error) {
	log := screenLog{"GLOBAL"}
	return log, nil
}

func (screenLogFactory) CreateSessionLog(sessionID quickfix.SessionID) (quickfix.Log, error) {
	log := screenLog{sessionID.String()}
	return log, nil
}

// NewFancyLog creates an instance of LogFactory that writes messages and events to stdout.
func NewFancyLog() quickfix.LogFactory {
	return screenLogFactory{}
}


func main() {

	fixApp := &FIXApplication{
		SessionIDs: make(map[string]quickfix.SessionID),
	}

	//logFactory := quickfix.NewScreenLogFactory()
	logFactory := NewFancyLog()

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

