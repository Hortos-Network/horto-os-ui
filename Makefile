# horto-os-ui developer targets
#
# Default members: horto-os-ui-shared, horto-os-ui-cli, horto-os-ui-tui, horto-os-ui-status-api, horto-os-ui-mcp
# Ops KPI (GPUI) is in the workspace but not default: use build-kpi / build-all / run-kpi.
# horto-os-ui-web (Leptos CSR) + horto-os-ui-desktop (Tauri) are PC-side; not in DEFAULT_PKGS.

SHELL := /bin/bash
ROOT := $(abspath .)
CARGO ?= cargo
# Scaffold still owes pedantic/nursery cleanup (missing_errors_doc / must_use flood).
# Keep -D warnings + clippy::all as the local ship gate until that pass lands.
CLIPPY_FLAGS := -D warnings -D clippy::all
RUSTDOCFLAGS ?= -D warnings

APP_VERSION ?= $(shell awk '/^version = /{gsub(/"/, "", $$3); print $$3; exit}' Cargo.toml)
PREFIX ?= $(HOME)/.local
BIN_DIR ?= $(PREFIX)/bin
TARGET_DIR ?= $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),$(ROOT)/target)
DOC_OUT ?= $(TARGET_DIR)/doc

# Default build set (excludes horto-os-ui-kpi / gpui).
DEFAULT_PKGS := -p horto-os-ui-shared -p horto-os-ui-cli -p horto-os-ui-tui -p horto-os-ui-status-api -p horto-os-ui-mcp

# Runtime helpers
API_BIND ?= 0.0.0.0:8787
HORTO_BOX_URL ?= http://localhost:8787
# Set DRY_RUN=0 (or APPLY=1) to drop --dry-run on CLI/TUI convenience targets.
DRY_RUN ?= 1
APPLY ?= 0
ARGS ?=
STEP ?= s1

# Paths filtered from llvm-cov / local shared fail-under. TUI draw is interactive.
# backup + s7 need root/host tools; kits/docker listing depends on a live daemon.
# (Codecov CI upload still uses a narrower ignore so those files stay visible there.)
COVERAGE_IGNORE := examples/|benches/|horto-os-ui-tui/src/main.rs|horto-os-ui-shared/src/ops/backup.rs|horto-os-ui-shared/src/steps/s7_activate.rs|horto-os-ui-shared/src/kits/docker.rs
# Shared engine gate (line %). Raise as coverage climbs; TUI/CLI stay out of this number.
COVERAGE_SHARED_FAIL_UNDER ?= 85

.DEFAULT_GOAL := help

.PHONY: help \
	build build-release build-kpi build-all check check-all \
	fmt format format-check clippy lint check-lint \
	test test-core test-all test-remote-docker verify ci \
	coverage coverage-summary coverage-shared coverage-html \
	audit deny machete outdated \
	doc doc-open doc-clean \
	run run-cli cli status doctor docker-status setup-run setup-step \
	backup-status backup-etc backup-disk-status \
	run-tui tui tui-release \
	run-api api \
	run-mcp mcp \
	run-kpi kpi \
	desktop-web desktop-web-serve build-desktop run-desktop desktop \
	release-bins release-checksums deb \
	install install-kpi uninstall \
	bins version-show clean \
	test-remote-docker \
	docker-build docker-run docker-build-mcp docker-run-mcp

# ---------------------------------------------------------------------------
# Help
# ---------------------------------------------------------------------------

help:
	@echo "horto-os-ui targets"
	@echo ""
	@echo "Build / check"
	@echo "  make build / build-release   default packages (cli, tui, api, mcp)"
	@echo "  make build-kpi               -p horto-os-ui-kpi (GPUI)"
	@echo "  make desktop                 Trunk + release Tauri app and open window"
	@echo "  make build-desktop           Trunk + release Tauri binary (no open)"
	@echo "  make desktop-web             Trunk release build of horto-os-ui-web"
	@echo "  make check / check-all       cargo check (default pkgs / workspace)"
	@echo "  make fmt / format            cargo fmt --check / cargo fmt"
	@echo "  make lint / check-lint       fmt check + clippy (default / fix)"
	@echo "  make test / test-core / test-all / test-remote-docker"
	@echo "  make verify / ci             lint + test + audit + deny + machete"
	@echo "  make coverage / coverage-summary / coverage-shared / coverage-html"
	@echo "  make audit / deny / machete / outdated"
	@echo "  make doc / doc-open          rustdoc → docs/api-rust/ (shared+cli+tui+api)"
	@echo ""
	@echo "Run (build then exec; cargo does not hold the lock)"
	@echo "  make run / run-cli / cli     horto-os-ui  ARGS='…'   (DRY_RUN=$(DRY_RUN))"
	@echo "  make status                  horto-os-ui --dry-run setup status"
	@echo "  make doctor                  horto-os-ui doctor"
	@echo "  make docker-status           horto-os-ui docker status"
	@echo "  make setup-run               horto-os-ui --dry-run setup run"
	@echo "  make setup-step STEP=s1      horto-os-ui --dry-run setup step \$$STEP"
	@echo "  make backup-status           horto-os-ui backup status"
	@echo "  make backup-etc              horto-os-ui --dry-run backup etc"
	@echo "  make backup-disk-status      horto-os-ui backup disk-status"
	@echo "  make run-tui / tui           horto-os-ui-tui --dry-run"
	@echo "  make tui-release             release binary, dry-run TUI"
	@echo "  make run-api / api           horto-os-ui-status-api  (API_BIND=$(API_BIND))"
	@echo "  make run-mcp / mcp           horto-os-ui-mcp stdio (MCP_HTTP=false)"
	@echo "  make run-kpi / kpi           horto-os-ui-kpi (HORTO_BOX_URL=$(HORTO_BOX_URL))"
	@echo "  make desktop-web-serve       Trunk serve web UI on :4187"
	@echo ""
	@echo "Install"
	@echo "  make install                 release binaries → $(BIN_DIR)"
	@echo "  make install-kpi             also horto-os-ui-kpi"
	@echo "  make release-bins            naked tar.gz + sha256 under dist/"
	@echo "  make docker-build / docker-run  status-api image (local tag)"
	@echo "  make docker-build-mcp / docker-run-mcp  MCP image (Cursor stdio)"
	@echo "  make deb                     .deb via cargo-deb (needs cargo install cargo-deb)"
	@echo "  make uninstall               remove installed horto* from $(BIN_DIR)"
	@echo "  make bins                    list built binaries under target/"
	@echo "  make version-show            print workspace version ($(APP_VERSION))"
	@echo "  make clean                   cargo clean"
	@echo ""
	@echo "Examples"
	@echo "  make cli ARGS='setup status --minimal'"
	@echo "  make cli DRY_RUN=0 ARGS='doctor'          # or APPLY=1"
	@echo "  make api API_BIND=127.0.0.1:8787"
	@echo "  make kpi HORTO_BOX_URL=http://192.168.1.10:8787"
	@echo ""
	@echo "Overrides: PREFIX CARGO_TARGET_DIR API_BIND HORTO_BOX_URL DRY_RUN APPLY ARGS STEP"

# ---------------------------------------------------------------------------
# Dry-run flag for CLI / TUI
# ---------------------------------------------------------------------------

dry_run_flag = $(if $(filter 1,$(APPLY)),,$(if $(filter 0,$(DRY_RUN)),,--dry-run))

# ---------------------------------------------------------------------------
# Build / check
# ---------------------------------------------------------------------------

build:
	cd $(ROOT) && $(CARGO) build $(DEFAULT_PKGS)

build-release:
	cd $(ROOT) && $(CARGO) build --release $(DEFAULT_PKGS)

build-kpi:
	cd $(ROOT) && $(CARGO) build -p horto-os-ui-kpi

build-all:
	cd $(ROOT) && $(CARGO) build -p horto-os-ui-shared -p horto-os-ui-cli -p horto-os-ui-tui \
		-p horto-os-ui-status-api -p horto-os-ui-mcp -p horto-os-ui-kpi

check:
	cd $(ROOT) && $(CARGO) check $(DEFAULT_PKGS) --all-targets

check-all:
	cd $(ROOT) && $(CARGO) check -p horto-os-ui-shared -p horto-os-ui-cli -p horto-os-ui-tui \
		-p horto-os-ui-status-api -p horto-os-ui-mcp -p horto-os-ui-kpi --all-targets

# ---------------------------------------------------------------------------
# Format / lint
# ---------------------------------------------------------------------------

fmt:
	cd $(ROOT) && $(CARGO) fmt --check

format:
	cd $(ROOT) && $(CARGO) fmt

format-check: fmt

clippy:
	cd $(ROOT) && $(CARGO) clippy $(DEFAULT_PKGS) --all-targets -- $(CLIPPY_FLAGS)

lint: fmt clippy

check-lint: format-check
	cd $(ROOT) && $(CARGO) clippy --fix --allow-dirty --allow-staged $(DEFAULT_PKGS) --all-targets -- $(CLIPPY_FLAGS)

# ---------------------------------------------------------------------------
# Test / verify
# ---------------------------------------------------------------------------

test:
	cd $(ROOT) && $(CARGO) test $(DEFAULT_PKGS)

test-core:
	cd $(ROOT) && $(CARGO) test -p horto-os-ui-shared

test-all:
	cd $(ROOT) && $(CARGO) test -p horto-os-ui-shared -p horto-os-ui-cli -p horto-os-ui-tui \
		-p horto-os-ui-status-api -p horto-os-ui-mcp -p horto-os-ui-kpi

# Live OpenSSH against a local Docker "box" (ignored unit is opted in here).
test-remote-docker: build
	cd $(ROOT) && $(CARGO) test -p horto-os-ui-shared --test remote_docker -- --ignored --nocapture

verify: lint test
	@echo "verify OK"

ci: lint test audit deny machete
	@echo "ci OK"

# ---------------------------------------------------------------------------
# Coverage (needs cargo-llvm-cov + llvm-tools-preview)
# ---------------------------------------------------------------------------

coverage:
	mkdir -p $(ROOT)/coverage
	cd $(ROOT) && RUSTUP_TOOLCHAIN=stable $(CARGO) llvm-cov $(DEFAULT_PKGS) --locked --lcov \
		--ignore-filename-regex '$(COVERAGE_IGNORE)' \
		--output-path coverage/lcov.info

coverage-summary:
	cd $(ROOT) && RUSTUP_TOOLCHAIN=stable $(CARGO) llvm-cov $(DEFAULT_PKGS) --locked --summary-only \
		--ignore-filename-regex '$(COVERAGE_IGNORE)'

# Shared engine only, with fail-under (primary quality gate for absorbable logic).
coverage-shared:
	mkdir -p $(ROOT)/coverage
	cd $(ROOT) && RUSTUP_TOOLCHAIN=stable $(CARGO) llvm-cov -p horto-os-ui-shared --locked --summary-only \
		--ignore-filename-regex '$(COVERAGE_IGNORE)' \
		--fail-under-lines $(COVERAGE_SHARED_FAIL_UNDER)

coverage-html:
	mkdir -p $(ROOT)/coverage
	cd $(ROOT) && RUSTUP_TOOLCHAIN=stable $(CARGO) llvm-cov $(DEFAULT_PKGS) --locked --html \
		--ignore-filename-regex '$(COVERAGE_IGNORE)' \
		--output-dir coverage/html

# ---------------------------------------------------------------------------
# Supply chain
# ---------------------------------------------------------------------------

## Advisory gate: deny.toml `[advisories] ignore` is the single allowlist
## (GPUI + Tauri transitive unmaintained crates). Do not duplicate IDs here.
audit:
	cd $(ROOT) && $(CARGO) deny check advisories

deny:
	@test -f $(ROOT)/deny.toml || (echo "missing deny.toml"; exit 1)
	cd $(ROOT) && $(CARGO) deny check

machete:
	cd $(ROOT) && $(CARGO) machete

outdated:
	cd $(ROOT) && $(CARGO) outdated --workspace

# ---------------------------------------------------------------------------
# Docs
# ---------------------------------------------------------------------------

# Libraries + bins that rustdoc can emit. KPI/desktop/web stay out (GPUI/Tauri/wasm).
DOC_PKGS := -p horto-os-ui-shared -p horto-os-ui-cli -p horto-os-ui-tui -p horto-os-ui-status-api -p horto-os-ui-mcp
DOC_CRATE ?= horto_os_ui_shared

## rustdoc → docs/api-rust/ (split-ready publish folder, same shape as other /opt2 products).
doc:
	cd $(ROOT) && RUSTDOCFLAGS='$(RUSTDOCFLAGS)' $(CARGO) doc $(DOC_PKGS) --no-deps --document-private-items
	@test -d "$(DOC_OUT)" || (echo "missing $(DOC_OUT)"; exit 1)
	@rm -rf $(ROOT)/docs/api-rust
	@mkdir -p $(ROOT)/docs/api-rust
	@cp -a "$(DOC_OUT)/." $(ROOT)/docs/api-rust/
	@printf '%s\n' \
		'<!DOCTYPE html>' \
		'<html lang="en">' \
		'<head>' \
		'<meta charset="utf-8">' \
		'<meta http-equiv="refresh" content="0; url=$(DOC_CRATE)/index.html">' \
		'<title>horto-os-ui - Rust API docs</title>' \
		'<link rel="canonical" href="$(DOC_CRATE)/index.html">' \
		'<script>location.replace("$(DOC_CRATE)/index.html");</script>' \
		'</head>' \
		'<body><p><a href="$(DOC_CRATE)/index.html">horto-os-ui-shared API documentation</a></p>' \
		'<p>Also: <a href="horto_os_ui/index.html">CLI</a>, ' \
		'<a href="horto_os_ui_tui/index.html">TUI</a>, ' \
		'<a href="horto_os_ui_status_api/index.html">status-api</a>, ' \
		'<a href="horto_os_ui_mcp/index.html">mcp</a>.</p></body>' \
		'</html>' \
		> $(ROOT)/docs/api-rust/index.html
	@printf '%s\n' \
		'# Rust API documentation (rustdoc)' \
		'' \
		'Generate with `make doc`, then open [`index.html`](index.html) (redirects to shared).' \
		'' \
		'| Crate | Path |' \
		'| ----- | ---- |' \
		'| Engine | [`horto_os_ui_shared/`](horto_os_ui_shared/index.html) |' \
		'| CLI | [`horto_os_ui/`](horto_os_ui/index.html) |' \
		'| TUI | [`horto_os_ui_tui/`](horto_os_ui_tui/index.html) |' \
		'| Status API | [`horto_os_ui_status_api/`](horto_os_ui_status_api/index.html) |' \
		'| MCP | [`horto_os_ui_mcp/`](horto_os_ui_mcp/index.html) |' \
		'' \
		'Operator docs: [`../README.md`](../README.md). Tip sync: [`../TIP_SYNC.md`](../TIP_SYNC.md).' \
		> $(ROOT)/docs/api-rust/README.md
	@touch $(ROOT)/docs/api-rust/.nojekyll
	@echo "docs/api-rust/ updated - open docs/api-rust/index.html"

doc-open: doc
	@xdg-open $(ROOT)/docs/api-rust/index.html 2>/dev/null \
		|| open $(ROOT)/docs/api-rust/index.html 2>/dev/null \
		|| echo "Open docs/api-rust/index.html in a browser"

doc-clean:
	rm -rf $(ROOT)/docs/api-rust
	mkdir -p $(ROOT)/docs/api-rust
	@printf '%s\n' \
		'# Rust API documentation (rustdoc)' \
		'' \
		'Generate with `make doc`, then open [`index.html`](index.html).' \
		> $(ROOT)/docs/api-rust/README.md

# ---------------------------------------------------------------------------
# Run: CLI
# ---------------------------------------------------------------------------

run: run-cli

run-cli: cli

cli:
	@cd $(ROOT) && $(CARGO) build -p horto-os-ui-cli -q
	@"$(TARGET_DIR)/debug/horto-os-ui" $(dry_run_flag) $(ARGS)

status:
	@$(MAKE) --no-print-directory cli ARGS='setup status $(ARGS)'

doctor:
	@$(MAKE) --no-print-directory cli DRY_RUN=0 ARGS='doctor $(ARGS)'

docker-status:
	@$(MAKE) --no-print-directory cli DRY_RUN=0 ARGS='docker status $(ARGS)'

setup-run:
	@$(MAKE) --no-print-directory cli ARGS='setup run $(ARGS)'

setup-step:
	@$(MAKE) --no-print-directory cli ARGS='setup step $(STEP) $(ARGS)'

backup-status:
	@$(MAKE) --no-print-directory cli DRY_RUN=0 ARGS='backup status $(ARGS)'

backup-etc:
	@$(MAKE) --no-print-directory cli ARGS='backup etc $(ARGS)'

backup-disk-status:
	@$(MAKE) --no-print-directory cli DRY_RUN=0 ARGS='backup disk-status $(ARGS)'

# ---------------------------------------------------------------------------
# Run: TUI
# ---------------------------------------------------------------------------

run-tui: tui

tui:
	@cd $(ROOT) && $(CARGO) build -p horto-os-ui-tui -q
	@exec "$(TARGET_DIR)/debug/horto-os-ui-tui" $(dry_run_flag) $(ARGS)

tui-release:
	@cd $(ROOT) && $(CARGO) build --release -p horto-os-ui-tui -q
	@exec "$(TARGET_DIR)/release/horto-os-ui-tui" $(dry_run_flag) $(ARGS)

# ---------------------------------------------------------------------------
# Run: API
# ---------------------------------------------------------------------------

run-api: api

api:
	@cd $(ROOT) && $(CARGO) build -p horto-os-ui-status-api -q
	@HORTO_API_BIND="$(API_BIND)" exec "$(TARGET_DIR)/debug/horto-os-ui-status-api" $(ARGS)

# ---------------------------------------------------------------------------
# Run: MCP (stdio by default)
# ---------------------------------------------------------------------------

run-mcp: mcp

mcp:
	@cd $(ROOT) && $(CARGO) build -p horto-os-ui-mcp -q
	@MCP_HTTP=false exec "$(TARGET_DIR)/debug/horto-os-ui-mcp" $(ARGS)

# ---------------------------------------------------------------------------
# Run: kpi (GPUI ops viewer)
# ---------------------------------------------------------------------------

run-kpi: kpi

kpi:
	@cd $(ROOT) && $(CARGO) build -p horto-os-ui-kpi
	@echo "starting $(TARGET_DIR)/debug/horto-os-ui-kpi → $(HORTO_BOX_URL)"
	@HORTO_BOX_URL="$(HORTO_BOX_URL)" exec "$(TARGET_DIR)/debug/horto-os-ui-kpi" $(ARGS)

# ---------------------------------------------------------------------------
# Run: desktop (Tauri + Leptos web UI)
# ---------------------------------------------------------------------------

desktop-web:
	cd $(ROOT)/crates/horto-os-ui-web && env -u NO_COLOR trunk build --release

desktop-web-serve:
	cd $(ROOT)/crates/horto-os-ui-web && env -u NO_COLOR trunk serve --release --port 4187 --address 127.0.0.1

run-desktop: desktop
## Build Trunk UI + Tauri shell (embeds frontendDist) and open the window.
## Needs `custom-protocol` (crate default). Without it Tauri hits build.devUrl.
desktop: desktop-web
	@echo "building horto-os-ui-desktop release (Tauri; first run can take minutes)..."
	cd $(ROOT)/crates/horto-os-ui-desktop && $(CARGO) build --release
	@bin="$(TARGET_DIR)/release/horto-os-ui-desktop"; \
	test -x "$$bin" || { echo "missing $$bin"; exit 1; }; \
	echo "starting $$bin"; \
	exec "$$bin" $(ARGS)

## Release Tauri binary only (no auto-open).
build-desktop: desktop-web
	cd $(ROOT)/crates/horto-os-ui-desktop && $(CARGO) build --release
	@echo "built $(TARGET_DIR)/release/horto-os-ui-desktop"

# ---------------------------------------------------------------------------
# Docker (status-api + MCP; local tags for smoke)
# ---------------------------------------------------------------------------

DOCKER_IMAGE ?= horto-os-ui-status-api:local
DOCKER_MCP_IMAGE ?= horto-os-ui-mcp:local

docker-build:
	docker build -f docker/Dockerfile -t "$(DOCKER_IMAGE)" "$(ROOT)"

docker-run:
	docker run --rm -p 8787:8787 "$(DOCKER_IMAGE)"

docker-build-mcp:
	docker build -f docker/Dockerfile.mcp -t "$(DOCKER_MCP_IMAGE)" "$(ROOT)"

docker-run-mcp:
	docker run --rm -i -e MCP_HTTP=false -e HORTO_MCP_MODE=pc "$(DOCKER_MCP_IMAGE)"

# ---------------------------------------------------------------------------
# Release packaging (naked tar.gz + optional .deb)
# ---------------------------------------------------------------------------

DIST_DIR ?= $(ROOT)/dist
HOST_TRIPLE ?= $(shell rustc -vV | awk '/^host:/{print $$2}')

release-bins: build-release
	@mkdir -p "$(DIST_DIR)"
	@set -e; \
	OUT="horto-os-ui-$(APP_VERSION)-$(HOST_TRIPLE)"; \
	STAGE="$(DIST_DIR)/$${OUT}"; \
	rm -rf "$${STAGE}"; \
	mkdir -p "$${STAGE}"; \
	cp "$(TARGET_DIR)/release/horto-os-ui" \
		"$(TARGET_DIR)/release/horto-os-ui-tui" \
		"$(TARGET_DIR)/release/horto-os-ui-status-api" \
		"$${STAGE}/"; \
	tar -C "$(DIST_DIR)" -czf "$(DIST_DIR)/$${OUT}.tar.gz" "$${OUT}"; \
	rm -rf "$${STAGE}"; \
	(cd "$(DIST_DIR)" && sha256sum "$${OUT}.tar.gz" > "$${OUT}.tar.gz.sha256"); \
	echo "wrote $(DIST_DIR)/$${OUT}.tar.gz"

release-checksums:
	@cd "$(DIST_DIR)" && \
		for f in horto-os-ui-$(APP_VERSION)-*.tar.gz *.deb; do \
			[ -f "$$f" ] || continue; \
			sha256sum "$$f" > "$$f.sha256"; \
			echo "checksum $$f.sha256"; \
		done

deb: build-release
	@command -v cargo-deb >/dev/null || { echo "install: cargo install cargo-deb"; exit 1; }
	@mkdir -p "$(DIST_DIR)"
	cd $(ROOT) && cargo deb -p horto-os-ui-cli --no-build --output "$(DIST_DIR)"
	cd $(ROOT) && cargo deb -p horto-os-ui-tui --no-build --output "$(DIST_DIR)"
	cd $(ROOT) && cargo deb -p horto-os-ui-status-api --no-build --output "$(DIST_DIR)"
	@$(MAKE) --no-print-directory release-checksums
	@echo "wrote .deb packages under $(DIST_DIR)"

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------

install: build-release
	@mkdir -p "$(BIN_DIR)"
	install -m 755 "$(TARGET_DIR)/release/horto-os-ui" "$(BIN_DIR)/horto-os-ui"
	install -m 755 "$(TARGET_DIR)/release/horto-os-ui-tui" "$(BIN_DIR)/horto-os-ui-tui"
	install -m 755 "$(TARGET_DIR)/release/horto-os-ui-status-api" "$(BIN_DIR)/horto-os-ui-status-api"
	install -m 755 "$(TARGET_DIR)/release/horto-os-ui-mcp" "$(BIN_DIR)/horto-os-ui-mcp"
	@echo "installed horto-os-ui horto-os-ui-tui horto-os-ui-status-api horto-os-ui-mcp → $(BIN_DIR)"

install-kpi: install
	cd $(ROOT) && $(CARGO) build --release -p horto-os-ui-kpi
	install -m 755 "$(TARGET_DIR)/release/horto-os-ui-kpi" "$(BIN_DIR)/horto-os-ui-kpi"
	@echo "installed horto-os-ui-kpi → $(BIN_DIR)"

uninstall:
	rm -f "$(BIN_DIR)/horto-os-ui" "$(BIN_DIR)/horto-os-ui-tui" "$(BIN_DIR)/horto-os-ui-status-api" "$(BIN_DIR)/horto-os-ui-mcp" "$(BIN_DIR)/horto-os-ui-kpi" "$(BIN_DIR)/horto-os-ui-desktop" "$(BIN_DIR)/horto" "$(BIN_DIR)/horto-tui" "$(BIN_DIR)/horto-kpi" "$(BIN_DIR)/horto-status-api" "$(BIN_DIR)/horto-api" "$(BIN_DIR)/horto-desktop" "$(BIN_DIR)/horto-mcp"
	@echo "removed horto* from $(BIN_DIR)"

bins:
	@echo "debug:"; ls -1 $(TARGET_DIR)/debug/horto-os-ui $(TARGET_DIR)/debug/horto-os-ui-tui $(TARGET_DIR)/debug/horto-os-ui-status-api $(TARGET_DIR)/debug/horto-os-ui-mcp $(TARGET_DIR)/debug/horto-os-ui-kpi 2>/dev/null || true
	@echo "release:"; ls -1 $(TARGET_DIR)/release/horto-os-ui $(TARGET_DIR)/release/horto-os-ui-tui $(TARGET_DIR)/release/horto-os-ui-status-api $(TARGET_DIR)/release/horto-os-ui-mcp $(TARGET_DIR)/release/horto-os-ui-kpi 2>/dev/null || true

version-show:
	@echo "workspace version: $(APP_VERSION)"
	@echo "packages: horto-os-ui-shared horto-os-ui-cli(horto-os-ui) horto-os-ui-tui horto-os-ui-status-api horto-os-ui-mcp horto-os-ui-kpi horto-os-ui-web horto-os-ui-desktop"

clean:
	cd $(ROOT) && $(CARGO) clean
