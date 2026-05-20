# Makefile to build camel_tokenizer.dll for SQLite FTS5 on Windows

SQLITE_INC ?= C:/msys64/mingw64/include
CC ?= gcc
CFLAGS ?= -O2 -Wall -I$(SQLITE_INC)
LDFLAGS ?= -shared

all: camel_tokenizer.dll

camel_tokenizer.dll: ../camel_tokenizer.c
	$(CC) $(CFLAGS) $(LDFLAGS) -o $@ $<

clean:
	del /Q camel_tokenizer.dll 2>NUL || true
