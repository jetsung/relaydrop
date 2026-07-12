FROM rust:1-trixie AS builder

WORKDIR /app

COPY . .

RUN cargo install --path .

FROM gcr.io/distroless/cc-debian13:latest
LABEL maintainer="Jetsung Chan<i@jetsung.com"

WORKDIR /app

COPY --from=builder /usr/local/cargo/bin/relaydrop /usr/local/bin/relaydrop

ENV RELAYDROP_LISTEN=0.0.0.0:9090

EXPOSE 9090

ENTRYPOINT ["relaydrop"]

CMD ["relay"]
