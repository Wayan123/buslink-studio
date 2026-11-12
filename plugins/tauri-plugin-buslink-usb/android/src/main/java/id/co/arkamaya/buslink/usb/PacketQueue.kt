package id.co.arkamaya.buslink.usb

import java.util.ArrayDeque
import java.util.concurrent.TimeUnit
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

/** Why a queue ended. */
enum class EndState(val wire: String) { DETACHED("detached"), CLOSED("closed") }

/** Result of [PacketQueue.poll]. */
sealed class PollResult {
    /** Bytes of one USB packet, or the first part of it when it was larger than the request. */
    class Data(val bytes: ByteArray) : PollResult()
    /** Nothing arrived in time. A timeout, not an error. */
    object Timeout : PollResult()
    class Ended(val state: EndState) : PollResult()
}

/**
 * Bounded hand-over between the USB reader thread and the Rust bridge.
 * Packets keep their order; a packet bigger than the caller's limit is
 * returned in pieces. When the queue is full the oldest bytes are dropped and
 * counted (nobody is reading, so the input is stale anyway).
 * No Android classes: unit-tested on the JVM.
 */
class PacketQueue(private val capacityBytes: Int = 8192) {
    private val lock = ReentrantLock()
    private val notEmpty = lock.newCondition()
    private val packets = ArrayDeque<ByteArray>()
    private var queued = 0
    private var ended: EndState? = null
    private var droppedBytes = 0L

    fun dropped(): Long = lock.withLock { droppedBytes }

    fun offer(packet: ByteArray) {
        if (packet.isEmpty()) return
        lock.withLock {
            if (ended != null) return
            var p = packet
            if (p.size > capacityBytes) {
                droppedBytes += (p.size - capacityBytes).toLong()
                p = p.copyOfRange(p.size - capacityBytes, p.size)
            }
            while (queued + p.size > capacityBytes) {
                val old = packets.pollFirst() ?: break
                queued -= old.size
                droppedBytes += old.size.toLong()
            }
            packets.addLast(p)
            queued += p.size
            notEmpty.signalAll()
        }
    }

    /** Ends the queue and discards what is queued (detach: buffered bytes are not trusted). */
    fun end(state: EndState) {
        lock.withLock {
            if (ended == null) ended = state
            packets.clear()
            queued = 0
            notEmpty.signalAll()
        }
    }

    fun poll(timeoutMs: Long, maxBytes: Int): PollResult {
        val limit = if (maxBytes < 1) 1 else maxBytes
        var nanos = TimeUnit.MILLISECONDS.toNanos(if (timeoutMs < 0) 0 else timeoutMs)
        lock.withLock {
            while (true) {
                ended?.let { return PollResult.Ended(it) }
                val head = packets.peekFirst()
                if (head != null) {
                    if (head.size <= limit) {
                        packets.pollFirst()
                        queued -= head.size
                        return PollResult.Data(head)
                    }
                    packets.pollFirst()
                    packets.addFirst(head.copyOfRange(limit, head.size))
                    queued -= limit
                    return PollResult.Data(head.copyOfRange(0, limit))
                }
                if (nanos <= 0L) return PollResult.Timeout
                nanos = notEmpty.awaitNanos(nanos)
            }
        }
    }
}
