import Foundation
import Network
import XCTest

extension XCTestCase {
    func verifyUSBServices(ports: [UInt16]) {
        let ready = expectation(description: "Both device-local fixture services respond over USB")
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 8
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel() }
        Task {
            defer { ready.fulfill() }
            do {
                let store = URL(string: "http://127.0.0.1:\(ports[1])/fixture-readiness")!
                let (_, response) = try await session.data(from: store)
                XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 404)
                let relay = session.webSocketTask(with: URL(string: "ws://127.0.0.1:\(ports[0])")!)
                relay.resume()
                defer { relay.cancel(with: .normalClosure, reason: nil) }
                try await relay.send(.string(#"["REQ","fixture-ready",{"ids":[]}]"#))
                guard case let .string(reply) = try await relay.receive(),
                      let values = try JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String],
                      values == ["EOSE", "fixture-ready"] else {
                    XCTFail("The USB relay did not answer its readiness subscription")
                    return
                }
            } catch {
                XCTFail("The physical USB fixture could not reach its services: \(error.localizedDescription)")
            }
        }
        wait(for: [ready], timeout: 12)
    }
}

/// Connect the two on-device fixture endpoints to their matching USB streams.
final class PhysicalLinkUSBFixture {
    private let queue = DispatchQueue(label: "physical-link-fixture")
    private let listeners: [NWListener]
    private let channels: [Channel]
    private let token: Data

    init(ports: [UInt16], token: String) throws {
        guard ports.count == 4, Set(ports).count == 4, ports.allSatisfy({ $0 > 1024 }),
              token.count == 48, token.allSatisfy({ $0.isHexDigit }) else {
            throw CocoaError(.coderInvalidValue)
        }
        self.token = Data(token.utf8)
        self.channels = [Channel(), Channel()]
        listeners = try ports.map { port in
            let parameters = NWParameters.tcp
            parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: port)!)
            return try NWListener(using: parameters)
        }
    }

    func start() {
        for (index, listener) in listeners.enumerated() {
            let channel = channels[index % 2]
            listener.newConnectionHandler = { [weak self] conn in
                guard let self, channel.connections.count < 48 else { conn.cancel(); return }
                let id = ObjectIdentifier(conn)
                channel.connections[id] = conn
                conn.stateUpdateHandler = { [weak conn] status in
                    switch status {
                    case .failed, .cancelled:
                        channel.connections.removeValue(forKey: id)
                        channel.clients.removeAll { ObjectIdentifier($0) == id }
                        channel.backends.removeAll { ObjectIdentifier($0) == id }
                        if case .failed = status { conn?.cancel() }
                    default: break
                    }
                }
                conn.start(queue: self.queue)
                if index < 2 {
                    channel.clients.append(conn)
                    self.match(channel)
                } else {
                    conn.receive(minimumIncompleteLength: self.token.count, maximumLength: self.token.count) { data, _, _, error in
                        guard error == nil, data == self.token else { conn.cancel(); return }
                        channel.backends.append(conn)
                        self.match(channel)
                    }
                }
            }
            listener.start(queue: queue)
        }
    }

    func stop() {
        queue.sync {
            listeners.forEach { $0.cancel() }
            channels.forEach { channel in channel.connections.values.forEach { $0.cancel() } }
        }
    }

    private func match(_ channel: Channel) {
        guard !channel.clients.isEmpty, !channel.backends.isEmpty else { return }
        let client = channel.clients.removeFirst()
        let backend = channel.backends.removeFirst()
        backend.send(content: Data([1]), completion: .contentProcessed { error in
            guard error == nil else { client.cancel(); backend.cancel(); return }
            var openDirections = 2 // Both callbacks run on the fixture's serial queue.
            let finished = {
                openDirections -= 1
                if openDirections == 0 { client.cancel(); backend.cancel() }
            }
            self.copy(client, to: backend, finished: finished)
            self.copy(backend, to: client, finished: finished)
        })
    }

    private func copy(_ source: NWConnection, to destination: NWConnection, finished: @escaping () -> Void) {
        source.receive(minimumIncompleteLength: 1, maximumLength: 32768) { data, _, complete, error in
            guard error == nil else { source.cancel(); destination.cancel(); return }
            destination.send(content: data, contentContext: complete ? .finalMessage : .defaultMessage,
                             isComplete: complete, completion: .contentProcessed { error in
                if error != nil { source.cancel(); destination.cancel() }
                else if complete { finished() }
                else { self.copy(source, to: destination, finished: finished) }
            })
        }
    }

    private final class Channel {
        var clients: [NWConnection] = []
        var backends: [NWConnection] = []
        var connections: [ObjectIdentifier: NWConnection] = [:]
    }
}
