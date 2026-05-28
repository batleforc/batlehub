package main

import (
	"net/http"
	"os"

	"github.com/gin-gonic/gin"
	"go.uber.org/zap"
)

func main() {
	log, _ := zap.NewProduction()
	defer log.Sync()

	r := gin.Default()
	r.GET("/", func(c *gin.Context) {
		c.JSON(http.StatusOK, gin.H{"message": "hello"})
	})

	port := os.Getenv("PORT")
	if port == "" {
		port = "8080"
	}
	log.Info("starting server", zap.String("addr", ":"+port))
	r.Run(":" + port)
}
