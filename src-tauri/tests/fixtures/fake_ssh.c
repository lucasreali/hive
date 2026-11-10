/* A stand-in for `ssh` in the app's tests (26.17): never a real host or `~/.ssh`. The tests
 * build it with `cc` into a folder of their own, which also holds:
 *
 * - `log`, to which each run appends its arguments, one line, separated by 0x1f;
 * - `plan`, whose line N says what the Nth run does (the last line holds for every later run):
 *   - `stderr <text>`: prints <text> (each `|` a new line) to stderr and exits 255, as `ssh`
 *     does for a refused host key, a failed login or a network error;
 *   - `sh <home> <path>`: runs the last argument (the remote command) with `/bin/sh -c`, as
 *     sshd runs it through the user's shell, with HOME=<home> and PATH=<path>;
 *   - `pipe <socket>`: connects stdin and stdout to the Unix socket <socket> (a test service);
 *   - `hang`: sleeps 30 s.
 *
 * `fake_ssh --pipe <socket>` pipes too, for a stand-in remote `hive bridge`. */
#include <errno.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

static int pipe_to(const char *path) {
    int sock = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un addr;
    memset(&addr, 0, sizeof addr);
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, path, sizeof addr.sun_path - 1);
    if (sock < 0 || connect(sock, (struct sockaddr *)&addr, sizeof addr) != 0) {
        fprintf(stderr, "fake_ssh: cannot connect to %s\n", path);
        return 255;
    }
    struct pollfd fds[2] = {{0, POLLIN, 0}, {sock, POLLIN, 0}};
    char buf[65536];
    for (;;) {
        if (poll(fds, 2, -1) < 0) {
            if (errno == EINTR) continue;
            return 255;
        }
        if (fds[0].revents) {
            ssize_t n = read(0, buf, sizeof buf);
            if (n <= 0) {
                shutdown(sock, SHUT_WR);
                fds[0].fd = -1;
            } else if (write(sock, buf, (size_t)n) != n) {
                return 255;
            }
        }
        if (fds[1].revents) {
            ssize_t n = read(sock, buf, sizeof buf);
            if (n <= 0) return 0;
            for (ssize_t done = 0; done < n;) {
                ssize_t w = write(1, buf + done, (size_t)(n - done));
                if (w <= 0) return 255;
                done += w;
            }
        }
    }
}

int main(int argc, char **argv) {
    if (argc == 3 && strcmp(argv[1], "--pipe") == 0) return pipe_to(argv[2]);
    char dir[4096];
    snprintf(dir, sizeof dir, "%s", argv[0]);
    char *slash = strrchr(dir, '/');
    if (slash) *slash = 0;
    char path[4200];
    snprintf(path, sizeof path, "%s/log", dir);
    int runs = 0;
    FILE *log = fopen(path, "r");
    if (log) {
        for (int c; (c = fgetc(log)) != EOF;) runs += c == '\n';
        fclose(log);
    }
    log = fopen(path, "a");
    for (int i = 1; i < argc; i++) fprintf(log, "%s%s", i > 1 ? "\x1f" : "", argv[i]);
    fprintf(log, "\n");
    fclose(log);
    snprintf(path, sizeof path, "%s/plan", dir);
    FILE *plan = fopen(path, "r");
    static char line[8192], step[8192];
    for (int i = 0; plan && i <= runs && fgets(line, sizeof line, plan); i++) {
        strcpy(step, line);
    }
    step[strcspn(step, "\n")] = 0;
    if (strncmp(step, "stderr ", 7) == 0) {
        for (char *c = step + 7; *c; c++) fputc(*c == '|' ? '\n' : *c, stderr);
        fputc('\n', stderr);
        return 255;
    }
    if (strncmp(step, "pipe ", 5) == 0) return pipe_to(step + 5);
    if (strcmp(step, "hang") == 0) {
        sleep(30);
        return 0;
    }
    if (strncmp(step, "sh ", 3) == 0) {
        char *home = step + 3, *space = strchr(home, ' ');
        if (!space) return 254;
        *space = 0;
        setenv("HOME", home, 1);
        setenv("PATH", space + 1, 1);
        unsetenv("HIVE_HOME");
        execl("/bin/sh", "sh", "-c", argv[argc - 1], (char *)0);
    }
    fprintf(stderr, "fake_ssh: no plan for run %d\n", runs + 1);
    return 254;
}
