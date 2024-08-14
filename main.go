package main

import (
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"strconv"
	"sync"

	pb "fixstatus/model"

	"github.com/quickfixgo/quickfix"
	"google.golang.org/protobuf/proto"
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
		log.Printf("FromApp function called")
		protoMsg, err := ConvertToProto(msg)
		if err != nil {
			log.Printf("Error converting FIX message to proto: %v", err)
			return
		}
	
		data, err := proto.Marshal(protoMsg)
		if err != nil {
			log.Printf("Failed to marshal proto message: %v", err)
			return
		}
	
		// Write the binary data to a file
		outputFile := "./output_messages.pb"
		file, err := os.OpenFile(outputFile, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
		if err != nil {
			log.Printf("Error opening output file: %v", err)
			return
		}
		defer file.Close()
	
		if _, err := file.Write(data); err != nil {
			log.Printf("Error writing proto message to file: %v", err)
			return
		}
	
		log.Printf("FIX message from session %s has been written to %s\n", sessionID, outputFile)
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

	http.HandleFunc("/protobuf-messages", serveProtoMessages)

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

func ConvertToProto(fixMsg *quickfix.Message) (*pb.FIXMessage, error) {
    protoMsg := &pb.FIXMessage{}

    // Extract FIX fields and populate the protobuf message
    if senderCompID, err := fixMsg.Header.GetString(quickfix.Tag(49)); err == nil {
        protoMsg.SenderCompId = senderCompID
    }
    if targetCompID, err := fixMsg.Header.GetString(quickfix.Tag(56)); err == nil {
        protoMsg.TargetCompId = targetCompID
    }
    if msgSeqNum, err := fixMsg.Header.GetInt(quickfix.Tag(34)); err == nil {
        protoMsg.MsgSeqNum = int32(msgSeqNum)
    }
    if msgType, err := fixMsg.Header.GetString(quickfix.Tag(35)); err == nil {
        protoMsg.MsgType = pb.MsgType(pb.MsgType_value[msgType])
    }
    if sendingTime, err := fixMsg.Header.GetString(quickfix.Tag(52)); err == nil {
        protoMsg.SendingTime = sendingTime
    }

    // Body fields specific to the message type (e.g., New Order - Single)
    if clOrdID, err := fixMsg.Body.GetString(quickfix.Tag(11)); err == nil {
        protoMsg.ClOrdId = clOrdID
    }
    if symbol, err := fixMsg.Body.GetString(quickfix.Tag(55)); err == nil {
        protoMsg.Symbol = symbol
    }
    if side, err := fixMsg.Body.GetString(quickfix.Tag(54)); err == nil {
        protoMsg.Side = pb.Side(pb.Side_value[string(side)])
    }
	if orderQtyStr, err := fixMsg.Body.GetString(quickfix.Tag(38)); err == nil {
		orderQty, err := strconv.ParseFloat(orderQtyStr, 64)
		if err != nil {
			log.Printf("Failed to parse OrderQty: %v", err)
		} else {
			protoMsg.OrderQty = orderQty
		}
	}
	if priceStr, err := fixMsg.Body.GetString(quickfix.Tag(44)); err == nil {
		price, err := strconv.ParseFloat(priceStr, 64)
		if err != nil {
			log.Printf("Failed to parse Price: %v", err)
		} else {
			protoMsg.Price = price
		}
	}
    if transactTime, err := fixMsg.Body.GetString(quickfix.Tag(60)); err == nil {
        protoMsg.TransactTime = transactTime
    }

    // Handle repeating group (e.g., NoContraBrokers)
    noContraBrokersGroup := quickfix.NewRepeatingGroup(quickfix.Tag(382),
        quickfix.GroupTemplate{
            quickfix.GroupElement(quickfix.Tag(375)),
            quickfix.GroupElement(quickfix.Tag(337)),
        },
    )
    if err := fixMsg.Body.GetGroup(noContraBrokersGroup); err == nil {
        for i := 0; i < noContraBrokersGroup.Len(); i++ {
            contraGroup := &pb.ContraBrokerGroup{}
            contraBrokers := noContraBrokersGroup.Get(i)
            if contraBroker, err := contraBrokers.GetString(quickfix.Tag(375)); err == nil {
                contraGroup.ContraBroker = contraBroker
            }
            if contraTrader, err := contraBrokers.GetString(quickfix.Tag(337)); err == nil {
                contraGroup.ContraTrader = contraTrader
            }
            protoMsg.ContraBrokers = append(protoMsg.ContraBrokers, contraGroup)
        }
    }

    return protoMsg, nil
}


func serveProtoMessages(w http.ResponseWriter, r *http.Request) {
	data, err := os.ReadFile("./output_messages.pb")
	if err != nil {
		http.Error(w, "Failed to read protobuf file", http.StatusInternalServerError)
		return
	}

	var protoMsg pb.FIXMessage
	err = proto.Unmarshal(data, &protoMsg)
	if err != nil {
		http.Error(w, "Failed to unmarshal protobuf data", http.StatusInternalServerError)
	}

	jsonData, err := json.Marshal(protoMsg)
	if err != nil {
		http.Error(w, "Failed to encode protobuf message as JSON", http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	w.Write(jsonData)
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
