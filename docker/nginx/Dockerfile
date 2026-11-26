FROM nginx:1.30.7-alpine

RUN addgroup -S saturn && adduser -S -u 1001 -G saturn saturn

COPY ./docker/nginx/nginx.conf /etc/nginx/nginx.conf

RUN touch /tmp/nginx.pid \
    && chown saturn:saturn /tmp/nginx.pid \
    && chown -R saturn:saturn /var/cache/nginx

USER saturn

EXPOSE 8080

CMD ["nginx", "-g", "daemon off;"]
