# HAL-9001 — alvos de conveniência para build, teste e execução.

.PHONY: check build test run

## check: compila sem gerar artefatos (detecção rápida de erros)
check:
	cargo check

## build: compila o binário de produção (release)
build:
	cargo build --release

## test: executa a suíte de testes (unit + integração)
test:
	cargo test

## run: executa a aplicação em modo de desenvolvimento
run:
	cargo run
