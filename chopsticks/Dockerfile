# Dockerfile
FROM node:20-alpine

# Set the working directory
WORKDIR /app

# Install curl and netcat for health checks and Chopsticks
RUN apk add --no-cache curl netcat-openbsd && \
    npm install -g @acala-network/chopsticks@latest

# Expose ports for both instances
EXPOSE 8000 8500 9000 9500

# Default command (this will be overridden by docker-compose)
CMD ["npx", "@acala-network/chopsticks", "-c", "/app/config.yml", "-p", "9000"]
