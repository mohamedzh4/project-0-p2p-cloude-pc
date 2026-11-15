DCL hardware test - instructions
================================

This test starts a small Linux virtual computer on your PC. Someone else on
the same Wi-Fi then opens a link and uses that virtual computer in their
browser.

The FIRST run downloads Ubuntu (about 600 MB) and installs a desktop inside
the virtual computer: allow 15-45 minutes. Later runs start in a few minutes.

The only program installed on your PC is QEMU (it runs the virtual
computer). Only devices on your own Wi-Fi/LAN can reach the virtual
computer, and only with the password shown. When you are done, the cleanup
step (see the end) removes everything the test added.

What you need
-------------
- A 64-bit PC with at least 2 CPU threads, about 3 GB free RAM and about
  9 GB free disk (the smallest test size).
- Virtualization turned on in BIOS/UEFI (Intel VT-x or AMD-V/SVM).
- Internet on this PC.
- A second device (phone, laptop) on the SAME Wi-Fi.

Choosing the size
-----------------
At the start, the test asks how big the virtual computer should be:

  CPU cores for the VM [1-7 cores, Enter = 2]:
  RAM for the VM [2-10 GiB, Enter = 2]:
  VM disk [6-40 GiB, Enter = 8]:

Press Enter to keep the smallest size that works, or type a bigger number
for a faster virtual computer. The range shown is what this PC can give
while keeping some for itself. The disk size is asked on the first run only.

Windows 10/11
-------------
1. Unzip this folder (right-click -> Extract All).
2. Double-click lan-test.cmd. Click "Yes" when Windows asks for permission.
3. If it says "Restart the PC", restart and double-click lan-test.cmd again.
4. Answer the size questions (Enter is fine), then wait. When it is ready it
   shows a link, a username and a password.

Linux (Ubuntu, Debian or Arch)
------------------------------
1. Unzip this folder.
2. Open a terminal in this folder and run:   bash lan-test.sh
3. Type your password if sudo asks. On Arch, if installing QEMU fails,
   run "sudo pacman -Syu" first, then run the test again.
4. Answer the size questions (Enter is fine), then wait. When it is ready it
   shows a link, a username and a password.

On the second device
--------------------
1. Open the link (https://192.168.x.x:8443) in Chrome, Edge or Firefox.
2. The browser warns "not private / not secure". This is expected for this
   test. Click "Advanced" then "Continue" / "Accept the risk".
3. Log in with username "ubuntu" and the password shown on the first PC.
4. You should see a desktop. Open the terminal or a few windows, try it.
5. Speed test: in the virtual computer's terminal, type  dcl-bench
   It takes about 2 minutes and saves the results in a file there too.

If the other device cannot open the link
----------------------------------------
- Use the link on the same network as the other device (the test may show
  more than one).
- A VPN on this PC can get in the way: turn it off for the test.
- Linux with a firewall other than ufw/firewalld: allow TCP port 8443.

If it says "did not answer"
---------------------------
Run it again: it reinstalls the desktop (takes as long as the first run).
Send the report file either way.

When you are done
-----------------
Press Ctrl+C in the test window (on Windows, then press Enter).
A file named lan-test-report-<date>.txt is saved in this folder.
Please send that file back. It contains no password.

Only checking the PC (no virtual computer):
  Windows:  lan-test.cmd -CheckOnly      (from a Command Prompt)
  Linux:    bash lan-test.sh --check

Removing everything afterwards
------------------------------
  Windows:  double-click lan-test-cleanup.cmd (click "Yes" on the prompt)
  Linux:    bash lan-test.sh --cleanup

It stops the virtual computer, removes the firewall rules, deletes the
virtual computer's files, and asks before removing QEMU (and on Windows,
before turning off Windows Hypervisor Platform). It only removes what the
test itself added. Your report files are kept; delete this folder after.
