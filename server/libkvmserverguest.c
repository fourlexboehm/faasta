#include <sys/types.h>

/* Resume storage VM with provided data shared two-ways. */
extern size_t sys_kvmserverguest_remote_resume(void* buffer, ssize_t len);
/* Wait for remote resume (in storage) */
extern size_t sys_kvmserverguest_storage_wait_paused(void** req, ssize_t len);

size_t kvmserverguest_remote_resume(void *buffer, ssize_t len) {
	return sys_kvmserverguest_remote_resume(buffer, len);
}

size_t kvmserverguest_storage_wait_paused(void** req, ssize_t len)
{
	return sys_kvmserverguest_storage_wait_paused(req, len);
}

asm(".global sys_kvmserverguest_remote_resume\n"
	".type sys_kvmserverguest_remote_resume, @function\n"
	"sys_kvmserverguest_remote_resume:\n"
	"	mov $0x10001, %eax\n"
	"	out %eax, $0\n"
	"   ret\n");

asm(".global sys_kvmserverguest_storage_wait_paused\n"
	".type sys_kvmserverguest_storage_wait_paused, @function\n"
	"sys_kvmserverguest_storage_wait_paused:\n"
	".cfi_startproc\n"
	"	mov $0x10002, %eax\n"
	"	out %eax, $0\n"
	"   wrfsbase %rdi\n"
	"	ret\n"
	".cfi_endproc\n");
