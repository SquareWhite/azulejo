import Foundation

// azj-observe: turns `herdr terminal session observe` NDJSON into terminal
// output. A terminal tile that another client owns runs
//
//   herdr terminal session observe <pane> --cols C --rows R | azj-observe
//
// as its child, so libghostty draws herdr's frames read-only. Each record is
// `{"type":"terminal.frame","bytes":"<base64 ANSI>",…}`; `terminal.closed`
// ends the stream. Anything else is ignored.

setvbuf(stdout, nil, _IONBF, 0)
let out = FileHandle.standardOutput

while let line = readLine(strippingNewline: true) {
    guard let data = line.data(using: .utf8),
          let record = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
          let type = record["type"] as? String
    else { continue }
    switch type {
    case "terminal.frame":
        if let encoded = record["bytes"] as? String, let bytes = Data(base64Encoded: encoded) {
            out.write(bytes)
        }
    case "terminal.closed":
        exit(0)
    default:
        continue
    }
}
