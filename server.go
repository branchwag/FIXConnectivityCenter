package main

import (
	"log"
	"net/http"
)

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