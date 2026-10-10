use tokio::io::BufReader;

use super::{InputLine, InputReader, InputRejection, MAX_INPUT_BYTES, read_input_line};

fn reader(bytes: Vec<u8>) -> InputReader<BufReader<std::io::Cursor<Vec<u8>>>> {
    InputReader::new(BufReader::new(std::io::Cursor::new(bytes)))
}

#[tokio::test]
async fn input_reader_handles_caps_framing_and_eof() {
    let exact = "a".repeat(MAX_INPUT_BYTES);
    let cases = [
        (format!("{exact}\n").into_bytes(), exact.clone()),
        (format!("{exact}\r\n").into_bytes(), exact.clone()),
        (exact.as_bytes().to_vec(), exact),
    ];

    for (bytes, expected) in cases {
        let mut input = reader(bytes);
        assert!(matches!(
            read_input_line(&mut input).await.expect("input should read"),
            InputLine::Prompt(prompt) if prompt == expected
        ));
        assert!(matches!(
            read_input_line(&mut input).await.expect("EOF should read"),
            InputLine::Eof
        ));
    }

    let mut input = reader(format!("{}\n", "a".repeat(MAX_INPUT_BYTES + 1)).into_bytes());
    assert!(matches!(
        read_input_line(&mut input)
            .await
            .expect("input should read"),
        InputLine::Rejected(InputRejection::TooLong)
    ));
}

#[tokio::test]
async fn input_reader_rejects_locally_and_drains_overflow_before_next_line() {
    let mut bytes = vec![b'a'; MAX_INPUT_BYTES + 10];
    bytes.extend_from_slice(b"\n  keep whitespace  \n\t \r\n");
    let mut input = reader(bytes);

    assert!(matches!(
        read_input_line(&mut input)
            .await
            .expect("oversized input should read"),
        InputLine::Rejected(InputRejection::TooLong)
    ));
    assert!(matches!(
        read_input_line(&mut input).await.expect("next line should read"),
        InputLine::Prompt(prompt) if prompt == "  keep whitespace  "
    ));
    assert!(matches!(
        read_input_line(&mut input)
            .await
            .expect("blank line should read"),
        InputLine::Rejected(InputRejection::Blank)
    ));

    let mut invalid = reader(vec![0xff, b'\n']);
    assert!(matches!(
        read_input_line(&mut invalid)
            .await
            .expect("invalid UTF-8 should read"),
        InputLine::Rejected(InputRejection::InvalidUtf8)
    ));
}
