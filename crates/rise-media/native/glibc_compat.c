#undef _GNU_SOURCE
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

long __isoc23_strtol(const char *text, char **end, int base) {
    return strtol(text, end, base);
}

long long __isoc23_strtoll(const char *text, char **end, int base) {
    return strtoll(text, end, base);
}

unsigned long __isoc23_strtoul(const char *text, char **end, int base) {
    return strtoul(text, end, base);
}

unsigned long long __isoc23_strtoull(const char *text, char **end, int base) {
    return strtoull(text, end, base);
}

int __isoc23_vsscanf(const char *text, const char *format, va_list arguments) {
    return vsscanf(text, format, arguments);
}

int __isoc23_sscanf(const char *text, const char *format, ...) {
    va_list arguments;
    va_start(arguments, format);
    int read = vsscanf(text, format, arguments);
    va_end(arguments);
    return read;
}
