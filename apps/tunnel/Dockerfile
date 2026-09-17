FROM alpine:3.19

RUN apk add --no-cache wireguard-tools iptables iproute2 bash curl ca-certificates

ARG WSTUNNEL_VERSION=10.7.1
RUN curl -sSL https://github.com/erebe/wstunnel/releases/download/v${WSTUNNEL_VERSION}/wstunnel_${WSTUNNEL_VERSION}_linux_amd64.tar.gz \
    | tar -xz -C /usr/local/bin wstunnel \
    && chmod +x /usr/local/bin/wstunnel

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
