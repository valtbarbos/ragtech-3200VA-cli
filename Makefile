COMPOSE_FILE := docker-compose.nobreak.yml
COMPOSE := docker compose -f $(COMPOSE_FILE)
STREAM_COMPOSE_FILE := docker-compose.nobreak.stream.yml
STREAM_COMPOSE := docker compose -p nobreak-stream -f $(STREAM_COMPOSE_FILE)

.PHONY: help up stream export status logs logs-stream down restart ps

help:
	@echo "Nobreak Docker stack commands"
	@echo "  make up       - Build and start exporter + loki + promtail + grafana"
	@echo "  make stream   - Isolated ndjson mode (stops export stack, starts nobreak-run only)"
	@echo "  make export   - Return to export stack (stops isolated nobreak-run first)"
	@echo "  make status   - Show service status"
	@echo "  make logs     - Tail logs from main services"
	@echo "  make logs-stream - Tail logs from isolated nobreak-run"
	@echo "  make down     - Stop and remove services"
	@echo "  make restart  - Restart all running services"
	@echo "  make ps       - Alias for status"
	@echo ""
	@echo "Optional env:"
	@echo "  NOBREAK_DEVICE=/dev/ttyACM0"

up:
	$(COMPOSE) up -d --build

stream:
	$(COMPOSE) stop nobreak-export || true
	$(STREAM_COMPOSE) up -d --build nobreak-run

export:
	$(STREAM_COMPOSE) down || true
	$(COMPOSE) up -d nobreak-export

status:
	$(COMPOSE) ps

ps: status

logs:
	$(COMPOSE) logs -f --tail=100 nobreak-export loki promtail grafana

logs-stream:
	$(STREAM_COMPOSE) logs -f --tail=100 nobreak-run

down:
	$(COMPOSE) down

restart:
	$(COMPOSE) restart
