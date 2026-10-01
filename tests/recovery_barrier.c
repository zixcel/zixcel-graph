/* Test-only: stop inside actual redb recovery after the target's real sync.
 * No header edits, substituted results, product switch or unrelated PID. */
#define _GNU_SOURCE
#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>
static int sync_barrier(int fd, long operation) {
    int result = (int)syscall(operation, fd), saved = errno;
    const char *expected = getenv("GRAPH_RECOVERY_FILE");
    static int reached = 0;
    if (result == 0 && expected && !reached) {
        char descriptor[64], path[PATH_MAX];
        snprintf(descriptor, sizeof(descriptor), "/proc/self/fd/%d", fd);
        ssize_t length = readlink(descriptor, path, sizeof(path)-1);
        if (length > 0) {
            path[length] = 0;
            if (strcmp(path, expected) == 0) {
                reached = 1;
                const char *action = getenv("GRAPH_RECOVERY_ACTION");
                if (action && strcmp(action, "fail") == 0) {
                    errno = EIO;
                    return -1;
                }
                const char marker[] = "GRAPH_RECOVERY_SYNC\n";
                if (write(1, marker, sizeof(marker)-1) != sizeof(marker)-1) _exit(91);
                char release;
                ssize_t n;
                do { n = read(0, &release, 1); } while (n < 0 && errno == EINTR);
                if (n != 1) _exit(92);
            }
        }
    }
    errno = saved;
    return result;
}
int fsync(int fd) { return sync_barrier(fd, SYS_fsync); }
int fdatasync(int fd) { return sync_barrier(fd, SYS_fdatasync); }
