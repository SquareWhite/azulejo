// Posts a distributed notification to the dev app, without any OS input:
//   swiftc -o notify notify.swift && ./notify dev.azulejo.debug.terminal "wake all"
import Foundation

let args = CommandLine.arguments
guard args.count == 3 else {
    FileHandle.standardError.write(Data("usage: notify <name> <object>\n".utf8))
    exit(2)
}
DistributedNotificationCenter.default().postNotificationName(
    Notification.Name(args[1]), object: args[2], userInfo: nil, deliverImmediately: true)
