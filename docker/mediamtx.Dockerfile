# The upstream image is scratch, so a healthcheck has nothing to run.
# A static busybox installed only as `wget` gives it an HTTP probe without a shell.
FROM busybox:1.38.0-musl@sha256:80e38f9a1b39188b8bdb28879178edb322a4402b081a9aedda1924609decd6f6 AS busybox
# The recording spool's mount point. Docker fills a new named volume from the image's
# directory, owner included, so the non-root user can write to it.
RUN mkdir /recordings

FROM bluenviron/mediamtx:1.21.3@sha256:59e43552f3be43695b70c19424dc2a42fcec18075f88d91561a24ac5d70a586c
COPY --from=busybox /bin/busybox /bin/wget
COPY --from=busybox --chown=65532:65532 /recordings /recordings
COPY deploy/mediamtx/mediamtx.yml /mediamtx.yml
USER 65532:65532
