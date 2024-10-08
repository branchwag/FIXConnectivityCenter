package main

import (
	"encoding/csv"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	pb "fixstatus/model"

	"github.com/quickfixgo/quickfix"
	"google.golang.org/protobuf/proto"
)

type FIXApplication struct {
	SessionStatus map[string]bool
	mu            sync.RWMutex
}

func ReadCSV(filePath string) ([]map[string]string, error) {
	file, err := os.Open(filePath)
	if err != nil {
		return nil, err
	}
	defer file.Close()

	reader := csv.NewReader(file)
	var records []map[string]string
	header, err := reader.Read()
	if err != nil {
		return nil, err
	}

	for {
		record, err := reader.Read()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, err
		}

		message := make(map[string]string)
		for i, value := range record {
			message[header[i]] = value
		}
		records = append(records, message)
	}

	return records, nil
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
	
		// Marshal the protobuf message to binary format
		data, err := proto.Marshal(protoMsg)
		if err != nil {
			log.Printf("Failed to marshal proto message: %v", err)
			return
		}
	
		go func() {
			conn, err := net.Dial("tcp", "localhost:9090")
			if err != nil {
				log.Printf("Failed to connect to TCP server: %v", err)
				return
			}
			defer conn.Close()

			if _, err := conn.Write(data); err != nil {
				log.Printf("Failed to send protobuf msg over TCP: %v", err)
				return
			}
			log.Printf("FIX msg from session %s has been streamed via TCP\n", sessionID)
		}()
		return
}

func SendFIXMessageFromCSV(fixApp *FIXApplication, sessionID quickfix.SessionID, data map[string]string) error {
	msg := quickfix.NewMessage()

	msgType := data["MsgType"]
	msg.Header.SetField(quickfix.Tag(35), quickfix.FIXString(msgType))

    senderCompID := data["SenderCompID"]
    targetCompID := data["TargetCompID"]
    msg.Header.SetField(quickfix.Tag(49), quickfix.FIXString(senderCompID))
    msg.Header.SetField(quickfix.Tag(56), quickfix.FIXString(targetCompID))

	// Remove these fields from the map as they are already set
	delete(data, "MsgType")
	delete(data, "SenderCompID")
	delete(data, "TargetCompID")

	// Set other FIX fields
	for tag, value := range data {
		tagNum, err := strconv.Atoi(tag)
		if err != nil {
			log.Printf("Invalid FIX tag: %s", tag)
			continue
		}
        // Assuming all fields in the CSV are string fields
        msg.Body.SetField(quickfix.Tag(tagNum), quickfix.FIXString(value))
	}

    // fixApp.mu.RLock()
    // sessionActive := fixApp.SessionStatus[sessionID.String()]
    // fixApp.mu.RUnlock()

    // if !sessionActive {
    //     return fmt.Errorf("failed to send FIX message: session %s is not active", sessionID)
    // }

    log.Printf("Attempting to send message to SessionID: %s\n", sessionID.String())


	// Send the message using the FIX session
	err := quickfix.SendToTarget(msg, sessionID)
	if err != nil {
		return fmt.Errorf("failed to send FIX message: %v", err)
	}

	log.Printf("Message sent: %v", msg)
	return nil
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

    // Wait for a logon to occur
    for {
        fixApp.mu.RLock()
        for sessionIDStr, connected := range fixApp.SessionStatus {
            if connected {
                fixApp.mu.RUnlock()

                // Manually construct SessionID from string
                sessionIDParts := strings.Split(sessionIDStr, ":")
                if len(sessionIDParts) != 2 {
                    log.Fatalf("Invalid SessionID format: %s", sessionIDStr)
                }

				beginString := sessionIDParts[0]
                senderTargetParts := strings.Split(sessionIDParts[1], "->")
                if len(senderTargetParts) != 2 {
                    log.Fatalf("Invalid SenderCompID and TargetCompID format: %s", sessionIDStr)
                }

                senderCompID := senderTargetParts[0]
                targetCompID := senderTargetParts[1]
                
                sid := quickfix.SessionID{
                    BeginString: beginString,
                    SenderCompID: senderCompID,
                    TargetCompID: targetCompID,
                }
                
                log.Printf("Constructed SessionID: %s", sid.String())

                csvData, err := ReadCSV("messages.csv")
                if err != nil {
                    log.Fatalf("Error reading CSV file: %v", err)
                }

                for _, record := range csvData {
                    err := SendFIXMessageFromCSV(fixApp, sid, record)
                    if err != nil {
                        log.Printf("Error sending FIX message: %v", err)
                    }
                }

              // Keep the session alive
			  for {
				fixApp.mu.RLock()
				active := fixApp.SessionStatus[sessionIDStr]
				fixApp.mu.RUnlock()
				if !active {
					log.Println("Session is no longer active. Exiting.")
					return
				}
				time.Sleep(30 * time.Second) // Wait and check again
            }
        }
	}
        fixApp.mu.RUnlock()
    }
}
	