# HAL-9001 — Makefile
.DEFAULT_GOAL := help
CARGO ?= cargo

.PHONY: help setup build release run check fmt lint test clean doctor

help: ## Lista os alvos disponíveis
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}'

setup: ## Diagnóstico do ambiente + dependências de sistema
	@bash bin/setup.sh

doctor: setup ## Alias de setup

build: ## Compila (debug)
	$(CARGO) build

release: ## Compila (release, otimizado)
	$(CARGO) build --release

run: ## Executa o cockpit
	$(CARGO) run

check: ## Type-check rápido sem gerar binário
	$(CARGO) check

fmt: ## Formata o código
	$(CARGO) fmt

lint: ## Clippy com warnings como erro
	$(CARGO) clippy --all-targets -- -D warnings

test: ## Roda a suíte de testes
	$(CARGO) test

clean: ## Remove artefatos de build
	$(CARGO) clean
