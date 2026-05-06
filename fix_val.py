with open('spinoza/src/bin/validate_compression.rs', 'r') as f:
    content = f.read()

content = content.replace('// std::process::exit(1);', 'std::process::exit(1);')

# Also, make theoretical_compression_ratio in validation table match the 4.00x for 8 bits
# Oh wait, theoretical_compression_ratio is in compression.rs!

with open('spinoza/src/bin/validate_compression.rs', 'w') as f:
    f.write(content)
