mod configuration;
mod control_packet;
mod data_packet;
mod multiplexer;
mod packet;
mod socket;
mod udt;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
