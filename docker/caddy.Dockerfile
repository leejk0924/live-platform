FROM node:26.13.0-trixie-slim@sha256:5adad64f9a1fca2d51972caa65983634c8a774eb300d72b20f7f2a0f60368072 AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# Source of the caddy binary and the MIME table only; the runtime image has no shell or package manager.
FROM caddy:2.11.8-alpine@sha256:f6228111e5fafa1a5238258d0858e103f10599b8d1659a146e7726681a002bb9 AS caddy

# A static busybox installed only as `wget` gives the healthcheck an HTTP probe without a shell.
FROM busybox:1.38.0-musl@sha256:80e38f9a1b39188b8bdb28879178edb322a4402b081a9aedda1924609decd6f6 AS busybox

FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab
ENV XDG_CONFIG_HOME=/config XDG_DATA_HOME=/data
COPY --from=caddy /usr/bin/caddy /usr/bin/caddy
COPY --from=caddy /etc/mime.types /etc/mime.types
COPY --from=busybox /bin/busybox /bin/wget
# Named volumes inherit this ownership on first mount, which lets the non-root user keep certificates in /data.
COPY --from=caddy --chown=65532:65532 /data /data
COPY --from=caddy --chown=65532:65532 /config /config
COPY deploy/caddy/Caddyfile /etc/caddy/Caddyfile
COPY --from=web /web/dist /srv
USER 65532:65532
WORKDIR /srv
EXPOSE 80 443 443/udp 2019
ENTRYPOINT ["/usr/bin/caddy"]
CMD ["run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"]
