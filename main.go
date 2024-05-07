package main

import (
	"fmt"
	"os"
)

func main() {
	_, err := os.Open("sessions.cfg")
	if err != nil {
		fmt.Println("Error opening config:", err)
		return
	}
}