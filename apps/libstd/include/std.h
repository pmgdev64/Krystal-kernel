#ifndef STD_H
#define STD_H
#include <stddef.h>
#include <stdint.h>
long std_read(int fd, void *buf, size_t n);
long std_write(int fd, const void *buf, size_t n);
long std_open(const char *path, int flags);
long std_close(int fd);
long std_lseek(int fd, long off, int whence);
long std_getcwd(char *buf, size_t n);
long std_chdir(const char *path);
long std_getdents64(int fd, void *buf, size_t n);
long std_dup2(int oldfd, int newfd);
long std_getpid(void);
void std_exit(int code) __attribute__((noreturn));
void *std_mmap_anon(size_t len);
long std_munmap(void *p, size_t len);
size_t std_strlen(const char *s);
int std_strcmp(const char *a, const char *b);
char *std_strcpy(char *d, const char *s);
void *std_memcpy(void *d, const void *s, size_t n);
void *std_memmove(void *d, const void *s, size_t n);
void *std_memset(void *d, int v, size_t n);
int std_memcmp(const void *a, const void *b, size_t n);
void std_puts(const char *s);
void std_putchar(unsigned char c);
void std_print_i64(long v);
void std_print_hex(unsigned long v);
void *std_malloc(size_t n);
void std_free(void *p);
#endif
