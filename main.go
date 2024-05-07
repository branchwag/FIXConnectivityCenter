package main

import (
	"fmt"
	"log"
	"os"

	"github.com/quickfixgo/quickfix"
)

type FIXApplication struct {
	SessionIDs map[string]quickfix.SessionID
}

func (a *FIXApplication) OnLogon(sessionID quickfix.SessionID){
	fmt.Printf("Session has logged on.")
}

func (a *FIXApplication) OnLogout(sessionID quickfix.SessionID){
	fmt.Printf("Logout.")
}

func (a *FIXApplication) ToAdmin(msg *quickfix.Message, sessionID quickfix.SessionID) {}

func (a *FIXApplication) OnCreate(sessionID quickfix.SessionID){
	a.SessionIDs[sessionID.String()] = sessionID
}

func (a *FIXApplication) FromAdmin(msg *quickfix.Message, sessionID quickfix.SessionID) (reject quickfix.MessageRejectError) {
	return
}

func (a *FIXApplication) ToApp(msg *quickfix.Message, sessionID quickfix.SessionID) (err error) {
	return
}

func (a *FIXApplication) FromApp(msg *quickfix.Message, sessionID quickfix.SessionID) (reject quickfix.MessageRejectError) {
	//handles application-level messages that are not recognized by QuickFIX/n as administrative or session-level messages.
    return
}


func main() {

	fixApp := &FIXApplication{
		SessionIDs: make(map[string]quickfix.SessionID),
	}

	logFactory := quickfix.NewScreenLogFactory()


	cfg, err := os.Open("sessions.cfg")
	if err != nil {
		fmt.Printf("Error opening config: %v", err)
		return
	}

	appSettings, err := quickfix.ParseSettings(cfg)
	fmt.Println(appSettings)
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

}

