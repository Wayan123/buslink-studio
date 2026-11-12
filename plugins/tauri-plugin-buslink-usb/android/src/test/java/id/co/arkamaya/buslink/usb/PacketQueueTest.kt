package id.co.arkamaya.buslink.usb

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PacketQueueTest {
    private fun bytes(vararg v: Int) = ByteArray(v.size) { v[it].toByte() }

    private fun data(r: PollResult): ByteArray = (r as PollResult.Data).bytes

    @Test
    fun timeoutIsReportedAsTimeoutNotAsData() {
        val q = PacketQueue()
        assertTrue(q.poll(10, 64) === PollResult.Timeout)
    }

    @Test
    fun fragmentedPacketsKeepOrder() {
        val q = PacketQueue()
        q.offer(bytes(1, 2))
        q.offer(bytes(3))
        q.offer(bytes(4, 5))
        assertArrayEquals(bytes(1, 2), data(q.poll(0, 64)))
        assertArrayEquals(bytes(3), data(q.poll(0, 64)))
        assertArrayEquals(bytes(4, 5), data(q.poll(0, 64)))
        assertTrue(q.poll(0, 64) === PollResult.Timeout)
    }

    @Test
    fun largePacketIsReturnedInPiecesWithoutLoss() {
        val q = PacketQueue()
        q.offer(bytes(1, 2, 3, 4, 5))
        assertArrayEquals(bytes(1, 2), data(q.poll(0, 2)))
        assertArrayEquals(bytes(3, 4), data(q.poll(0, 2)))
        assertArrayEquals(bytes(5), data(q.poll(0, 2)))
    }

    @Test
    fun overflowDropsOldestAndCounts() {
        val q = PacketQueue(4)
        q.offer(bytes(1, 2, 3))
        q.offer(bytes(4, 5, 6))
        assertEquals(3L, q.dropped())
        assertArrayEquals(bytes(4, 5, 6), data(q.poll(0, 64)))
    }

    @Test
    fun detachDiscardsQueuedBytesAndEnds() {
        val q = PacketQueue()
        q.offer(bytes(1, 2, 3))
        q.end(EndState.DETACHED)
        val r = q.poll(0, 64)
        assertTrue(r is PollResult.Ended && r.state == EndState.DETACHED)
        q.offer(bytes(9))
        assertTrue(q.poll(0, 64) is PollResult.Ended)
    }

    @Test
    fun blockedPollWakesWhenAPacketArrives() {
        val q = PacketQueue()
        val t = Thread {
            Thread.sleep(30)
            q.offer(bytes(7))
        }
        t.start()
        val r = q.poll(2000, 64)
        t.join()
        assertArrayEquals(bytes(7), data(r))
    }

    @Test
    fun hexRoundTrip() {
        val b = bytes(0x01, 0x03, 0xAB, 0x00, 0xFF)
        assertEquals("0103ab00ff", Hex.encode(b))
        assertArrayEquals(b, Hex.decode("0103AB00FF"))
        assertNull(Hex.decode("abc"))
        assertNull(Hex.decode("zz"))
    }
}
