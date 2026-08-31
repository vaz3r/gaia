FROM alpine:latest
RUN apk add --no-cache postgresql16-client rclone bash tzdata jq

WORKDIR /app
COPY backup.sh /app/backup.sh
RUN chmod +x /app/backup.sh

# Run once on startup, then crond in foreground
CMD /app/backup.sh && echo "${BACKUP_CRON:-0 2 * * *} /app/backup.sh" > /etc/crontabs/root && crond -f -d 8
