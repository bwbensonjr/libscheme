#
# Makefile for libscheme
#

#
# This must be an ANSI C compiler.
#
CC=cc

#
# Optimization and debugging flags go here.
#
CFLAGS=-O

# Path for Boehm garbage collector files.
#
BDW_PATH=/opt/homebrew/opt/bdw-gc

# Include files and lib path
#
INCLUDE=-I$(BDW_PATH)/include
LIB_PATH=-L. -L$(BDW_PATH)/lib

# The math library is needed for the numeric functions
# in scheme_number.c we need the Boehm GC.
#
LIBS=-lm -lgc

all: libscheme.dylib scheme

OBJS =  scheme_alloc.o \
	scheme_bool.o \
	scheme_char.o \
	scheme_env.o \
	scheme_error.o \
	scheme_eval.o \
	scheme_fun.o \
	scheme_hash.o \
	scheme_list.o \
	scheme_number.o \
	scheme_port.o \
	scheme_print.o \
	scheme_promise.o \
	scheme_read.o \
	scheme_string.o \
	scheme_struct.o \
	scheme_symbol.o \
	scheme_syntax.o \
	scheme_type.o \
	scheme_vector.o

SRCS =  scheme_alloc.c \
	scheme_bool.c \
	scheme_char.c \
	scheme_env.c \
	scheme_error.c \
	scheme_eval.c \
	scheme_fun.c \
	scheme_hash.c \
	scheme_list.c \
	scheme_number.c \
	scheme_port.c \
	scheme_print.c \
	scheme_promise.c \
	scheme_read.c \
	scheme_string.c \
	scheme_struct.c \
	scheme_symbol.c \
	scheme_syntax.c \
	scheme_type.c \
	scheme_vector.c

libscheme.dylib: $(OBJS)
	$(CC) -dynamiclib -o libscheme.dylib $(OBJS)

scheme: libscheme.dylib main.o 
	$(CC) $(CFLAGS) -o scheme main.o $(LIB_PATH) -lscheme -lm

clean:
	/bin/rm -f $(OBJS) main.o libscheme.dylib scheme
