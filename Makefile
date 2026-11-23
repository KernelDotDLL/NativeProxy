# NativeRoxy build tasks.
#
#   make            build Linux and Windows release binaries
#   make linux      build the Linux target
#   make windows    cross-build the Windows target
#   make linux-musl build a static Linux binary (optional)
#   make native     build for the machine running make
#   make dist       copy Linux and Windows binaries into dist/
#   make check      fmt --check + clippy + tests
#   make setup      add the cross targets and print linker instructions
#   make help       show this list

CARGO   ?= cargo
RUSTUP  ?= rustup
BIN     ?= NativeRoxy

TARGET_LINUX      ?= x86_64-unknown-linux-gnu
TARGET_LINUX_MUSL ?= x86_64-unknown-linux-musl
TARGET_WINDOWS    ?= x86_64-pc-windows-gnu

MINGW_CC ?= x86_64-w64-mingw32-gcc

WINDOWS_LINKER_ENV := CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER

PROFILE  ?= release
DIST_DIR ?= dist

ifeq ($(PROFILE),release)
CARGO_FLAGS := --release
PROFILE_DIR := release
else
CARGO_FLAGS :=
PROFILE_DIR := debug
endif

LINUX_BIN  := target/$(TARGET_LINUX)/$(PROFILE_DIR)/$(BIN)
MUSL_BIN   := target/$(TARGET_LINUX_MUSL)/$(PROFILE_DIR)/$(BIN)
WINDOWS_BIN := target/$(TARGET_WINDOWS)/$(PROFILE_DIR)/$(BIN).exe

.DEFAULT_GOAL := all
.DELETE_ON_ERROR:
.PHONY: all linux linux-musl windows check-windows native dist setup test lint fmt fmt-check check clean help

all: linux windows

linux:
	$(CARGO) build $(CARGO_FLAGS) --target $(TARGET_LINUX)
	@echo "built $(LINUX_BIN)"

linux-musl:
	$(CARGO) build $(CARGO_FLAGS) --target $(TARGET_LINUX_MUSL)
	@echo "built $(MUSL_BIN)"

check-windows:
	@command -v $(RUSTUP) >/dev/null 2>&1 || { \
		echo "error: rustup not found"; exit 1; \
	}
	@$(RUSTUP) target list --installed | grep -qx '$(TARGET_WINDOWS)' || { \
		echo "error: $(TARGET_WINDOWS) is not installed."; \
		echo "       run: make setup"; \
		exit 1; \
	}
	@command -v $(MINGW_CC) >/dev/null 2>&1 || { \
		echo "error: $(MINGW_CC) not found."; \
		echo "       Debian/Ubuntu: sudo apt install mingw-w64"; \
		echo "       Fedora:        sudo dnf install mingw64-gcc"; \
		echo "       Arch:          sudo pacman -S mingw-w64-gcc"; \
		echo "       macOS:         brew install mingw-w64"; \
		exit 1; \
	}

windows: check-windows
	env "$(WINDOWS_LINKER_ENV)=$(MINGW_CC)" \
		$(CARGO) build $(CARGO_FLAGS) --target $(TARGET_WINDOWS)
	@echo "built $(WINDOWS_BIN)"

native:
	$(CARGO) build $(CARGO_FLAGS)
	@echo "built $(BIN) in target/$(PROFILE_DIR)/"

dist: all
	@mkdir -p $(DIST_DIR)
	cp $(LINUX_BIN) $(DIST_DIR)/$(BIN)-$(TARGET_LINUX)
	cp $(WINDOWS_BIN) $(DIST_DIR)/$(BIN)-$(TARGET_WINDOWS).exe
	@ls -lh $(DIST_DIR)

test:
	$(CARGO) test

lint:
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	$(CARGO) fmt

fmt-check:
	$(CARGO) fmt --check

check: fmt-check lint test

setup:
	$(RUSTUP) target add $(TARGET_WINDOWS) $(TARGET_LINUX_MUSL)
	@echo
	@echo "The Windows cross-build also needs a MinGW linker:"
	@echo "  Debian/Ubuntu: sudo apt install mingw-w64"
	@echo "  Fedora:        sudo dnf install mingw64-gcc"
	@echo "  Arch:          sudo pacman -S mingw-w64-gcc"
	@echo "  macOS:         brew install mingw-w64"
	@echo "  Windows:       use 'make native' with the MSVC toolchain"

clean:
	$(CARGO) clean
	rm -rf $(DIST_DIR)

help:
	@echo "Targets:"
	@echo "  all         build Linux and Windows release binaries"
	@echo "  linux       build $(TARGET_LINUX)"
	@echo "  linux-musl  build $(TARGET_LINUX_MUSL) (static)"
	@echo "  windows     cross-build $(TARGET_WINDOWS)"
	@echo "  native      build for the host that runs make"
	@echo "  dist        copy Linux and Windows binaries into $(DIST_DIR)/"
	@echo "  test        cargo test"
	@echo "  lint        cargo clippy --all-targets -- -D warnings"
	@echo "  fmt         cargo fmt"
	@echo "  check       fmt --check, clippy, tests"
	@echo "  setup       add the cross targets and print linker instructions"
	@echo "  clean       remove target/ and dist/"
